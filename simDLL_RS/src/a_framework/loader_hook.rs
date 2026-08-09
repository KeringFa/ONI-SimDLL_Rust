//! mod 加载器：原生 detour 安装（RS_LoaderInstall）。
//!
//! 背景：SimDLL_Rust 托管 mod 无法用 Harmony patch 游戏的 extern（P/Invoke）
//! 方法——当前游戏版本的 Harmony 对 extern 生成 wrapper 时抛
//! InvalidProgramException（U59 实测崩溃）。因此改走原生 detour：本 DLL
//! （由接口 mod LoadLibrary 加载后）把原版 SimDLL.dll 的 16 个导出函数入口
//! 改写为 12 字节绝对跳转（`48 B8 <imm64>` mov rax, dest；`FF E0` jmp rax），
//! 重定向到本 DLL 的同名实现。我们不调用原版函数，因此不需要 trampoline
//! （不保存原指令）。
//!
//! 安全性：安装时机为 mod OnLoad（主菜单前，sim 未启动、无并发调用）；
//! 覆盖 12 字节后用 VirtualProtect 改页属性 + FlushInstructionCache 刷指令
//! 缓存。原版 SimDLL.dll 由 Unity 正常加载但入口被接管，永不执行。

use std::os::raw::c_void;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use windows::core::{PCSTR, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleA, GetModuleHandleW, GetProcAddress, LoadLibraryW,
};
use windows::Win32::System::Memory::{
    VirtualProtect, PAGE_EXECUTE, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};
use windows::Win32::System::Threading::GetCurrentProcess;

use crate::a_framework::sim_api;
use crate::c_simulation::conduit_temperature;

/// 一个接管点：原版导出名（NUL 结尾）+ 本 DLL 对应实现地址。
struct HookSpec {
    name: &'static [u8],
    target: usize,
}

const fn hook(name: &'static [u8], target: usize) -> HookSpec {
    HookSpec { name, target }
}

/// 安装全部接管点。返回成功安装数；失败返回负错误码。
#[no_mangle]
pub extern "C" fn RS_LoaderInstall() -> i32 {
    // 注意：这里不初始化 logger——日志默认关闭（接口 mod 配置 "off" 时不创建
    // 日志文件）；仅当接口 mod 通过 RS_SetLogEnabled(true) 启用时才惰性初始化。
    // 无 subscriber 时 tracing 宏为 no-op，安装结果可安全丢弃。
    let result = unsafe { install_all() };
    tracing::info!(result, "RS_LoaderInstall done");
    result
}

/// 16 个被接管的导出：Sim API（9）+ ConduitTemperatureManager（7），
/// 与接口 mod Native.cs 的 DllImport 声明一一对应。
fn hook_specs() -> [HookSpec; 16] {
    [
        hook(b"SIM_Initialize\0", sim_api::SIM_Initialize as usize),
        hook(b"SIM_Shutdown\0", sim_api::SIM_Shutdown as usize),
        hook(b"SIM_HandleMessage\0", sim_api::SIM_HandleMessage as usize),
        hook(b"SIM_HandleMessages\0", sim_api::SIM_HandleMessages as usize),
        hook(b"SIM_BeginSave\0", sim_api::SIM_BeginSave as usize),
        hook(b"SIM_EndSave\0", sim_api::SIM_EndSave as usize),
        hook(b"SIM_DebugCrash\0", sim_api::SIM_DebugCrash as usize),
        hook(b"SYSINFO_Acquire\0", sim_api::SYSINFO_Acquire as usize),
        hook(b"SYSINFO_Release\0", sim_api::SYSINFO_Release as usize),
        hook(
            b"ConduitTemperatureManager_Initialize\0",
            conduit_temperature::ConduitTemperatureManager_Initialize as usize,
        ),
        hook(
            b"ConduitTemperatureManager_Shutdown\0",
            conduit_temperature::ConduitTemperatureManager_Shutdown as usize,
        ),
        hook(
            b"ConduitTemperatureManager_Add\0",
            conduit_temperature::ConduitTemperatureManager_Add as usize,
        ),
        hook(
            b"ConduitTemperatureManager_Set\0",
            conduit_temperature::ConduitTemperatureManager_Set as usize,
        ),
        hook(
            b"ConduitTemperatureManager_Remove\0",
            conduit_temperature::ConduitTemperatureManager_Remove as usize,
        ),
        hook(
            b"ConduitTemperatureManager_Update\0",
            conduit_temperature::ConduitTemperatureManager_Update as usize,
        ),
        hook(
            b"ConduitTemperatureManager_Clear\0",
            conduit_temperature::ConduitTemperatureManager_Clear as usize,
        ),
    ]
}

unsafe fn install_all() -> i32 {
    // 定位原版 SimDLL.dll 模块：Unity 对原生插件是惰性加载（首次 DllImport
    // 才 LoadLibrary），mod OnLoad 时 sim 尚未启动、原版可能还未入进程，
    // 因此先查已加载模块；找不到则按游戏目录完整路径显式加载。
    let original = match locate_original_module() {
        Some(h) => h,
        None => {
            tracing::error!("failed to locate original SimDLL.dll");
            return -1;
        }
    };

    let mut installed = 0i32;
    for spec in hook_specs().iter() {
        match GetProcAddress(original, PCSTR(spec.name.as_ptr())) {
            Some(entry) => {
                if install_absolute_jump(entry as *mut c_void, spec.target as *const c_void) {
                    installed += 1;
                    tracing::trace!(
                        export = ?String::from_utf8_lossy(&spec.name[..spec.name.len() - 1]),
                        "hook installed"
                    );
                } else {
                    tracing::error!(
                        export = ?String::from_utf8_lossy(&spec.name[..spec.name.len() - 1]),
                        "hook install failed"
                    );
                }
            }
            None => {
                tracing::warn!(
                    export = ?String::from_utf8_lossy(&spec.name[..spec.name.len() - 1]),
                    "export not found in original SimDLL.dll"
                );
            }
        }
    }
    installed
}

/// 定位原版 SimDLL.dll：优先已加载模块；否则用 exe 目录拼
/// `OxygenNotIncluded_Data\Plugins\x86_64\SimDLL.dll` 完整路径 LoadLibrary。
/// 显式加载后，游戏 DllImport("SimDLL") 首次解析会命中该已加载模块
/// （按文件名匹配），入口已被接管。
unsafe fn locate_original_module() -> Option<HMODULE> {
    if let Ok(h) = GetModuleHandleA(PCSTR(b"SimDLL.dll\0".as_ptr())) {
        return Some(h);
    }

    // exe 路径（GetModuleFileNameW(NULL)）→ 目录 → Plugins\x86_64\SimDLL.dll
    let mut buf = [0u16; 1024];
    let len = GetModuleFileNameW(GetModuleHandleW(None).ok()?, &mut buf);
    if len == 0 {
        return None;
    }
    let exe = String::from_utf16_lossy(&buf[..len as usize]);
    let exe_dir = std::path::Path::new(&exe).parent()?.to_path_buf();
    let candidate = exe_dir
        .join("OxygenNotIncluded_Data")
        .join("Plugins")
        .join("x86_64")
        .join("SimDLL.dll");
    if !candidate.exists() {
        tracing::error!(path = ?candidate, "original SimDLL.dll not found on disk");
        return None;
    }
    let wide: Vec<u16> = candidate
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    match LoadLibraryW(PCWSTR(wide.as_ptr())) {
        Ok(h) => {
            tracing::info!(path = ?candidate, "loaded original SimDLL.dll");
            Some(h)
        }
        Err(e) => {
            tracing::error!(path = ?candidate, error = ?e, "LoadLibraryW failed");
            None
        }
    }
}

/// 在目标函数入口写 12 字节绝对跳转：
/// `48 B8 <imm64>` mov rax, dest；`FF E0` jmp rax。
/// 返回 false 表示 VirtualProtect/写入失败。
unsafe fn install_absolute_jump(target: *mut c_void, dest: *const c_void) -> bool {
    const JUMP_SIZE: usize = 12;
    let mut old_protect = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(
        target as *const c_void,
        JUMP_SIZE,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    )
    .is_err()
    {
        return false;
    }

    let bytes = target as *mut u8;
    *bytes.add(0) = 0x48; // mov rax, imm64
    *bytes.add(1) = 0xB8;
    std::ptr::write_unaligned(bytes.add(2) as *mut u64, dest as u64);
    *bytes.add(10) = 0xFF; // jmp rax
    *bytes.add(11) = 0xE0;

    let _ = VirtualProtect(target as *const c_void, JUMP_SIZE, PAGE_EXECUTE, &mut old_protect);
    let _ = FlushInstructionCache(GetCurrentProcess(), Some(target), JUMP_SIZE);
    true
}
