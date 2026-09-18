//! mod 加载器：原生 detour 安装（RS_LoaderInstall）。
//!
//! 背景：SimDLL_Rust 托管 mod 无法用 Harmony patch 游戏的 extern（P/Invoke）
//! 方法——当前游戏版本的 Harmony 对 extern 生成 wrapper 时抛
//! InvalidProgramException（U59 实测崩溃）。因此改走原生 detour：本 DLL
//! （由接口 mod LoadLibrary/dlopen 加载后）把原版 SimDLL 的 16 个导出函数入口
//! 改写为 12 字节绝对跳转（`48 B8 <imm64>` mov rax, dest；`FF E0` jmp rax），
//! 重定向到本 DLL 的同名实现。我们不调用原版函数，因此不需要 trampoline
//! （不保存原指令）。
//!
//! 安全性：安装时机为 mod OnLoad（主菜单前，sim 未启动、无并发调用）；
//! Windows 用 VirtualProtect 改页属性 + FlushInstructionCache 刷指令缓存；
//! Linux 用 mprotect（页对齐）——x86_64 自修改代码硬件保证指令缓存一致，
//! 无需显式刷缓存。原版 SimDLL 保持原位但入口被接管，永不执行。
//!
//! 平台差异（2026-09-09 Linux 兼容）：
//! - Windows：GetModuleHandle/LoadLibrary 定位 SimDLL.dll → GetProcAddress 取入口
//!   → VirtualProtect(RWX) 写跳转 → 还原保护。
//! - Linux：dlopen(NOLOAD→显式) 定位 libSimDLL.so（兼容 SimDLL.so）→ dlsym 取入口
//!   → mprotect(RWX 页对齐) 写跳转 → 还原 R|X。
//! 两者都不调用原版函数——只取入口地址。

use std::os::raw::c_void;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows::core::{PCSTR, PCWSTR};
#[cfg(windows)]
use windows::Win32::Foundation::HMODULE;
#[cfg(windows)]
use windows::Win32::System::Diagnostics::Debug::FlushInstructionCache;
#[cfg(windows)]
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleA, GetModuleHandleW, GetProcAddress, LoadLibraryW,
};
#[cfg(windows)]
use windows::Win32::System::Memory::{
    VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};
#[cfg(windows)]
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

/// 追踪一个安装结果（平台无关）。
fn trace_installed(name: &[u8], ok: bool) {
    let name = String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]);
    if ok {
        tracing::trace!(export = ?name, "hook installed");
    } else {
        tracing::error!(export = ?name, "hook install failed");
    }
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

// ---------------------------------------------------------------------------
// Windows 实现
// ---------------------------------------------------------------------------
#[cfg(windows)]
unsafe fn install_all() -> i32 {
    let mut installed = 0i32;
    for spec in hook_specs().iter() {
        if let Some(entry) = resolve_original_export(spec.name) {
            let ok = install_absolute_jump(entry, spec.target as *const c_void);
            trace_installed(spec.name, ok);
            if ok {
                installed += 1;
            }
        }
    }
    installed
}

/// 查找原版 SimDLL 模块中名为 `name` 的导出函数地址。
#[cfg(windows)]
unsafe fn resolve_original_export(name: &[u8]) -> Option<*mut c_void> {
    // 定位原版 SimDLL.dll 模块：Unity 对原生插件是惰性加载（首次 DllImport
    // 才 LoadLibrary），mod OnLoad 时 sim 尚未启动、原版可能还未入进程，
    // 因此先查已加载模块；找不到则按游戏目录完整路径显式加载。
    let original = match locate_original_module() {
        Some(h) => h,
        None => {
            tracing::error!("failed to locate original SimDLL.dll");
            return None;
        }
    };
    match GetProcAddress(original, PCSTR(name.as_ptr())) {
        Some(entry) => Some(entry as *mut c_void),
        None => {
            tracing::warn!(
                export = ?String::from_utf8_lossy(&name[..name.len() - 1]),
                "export not found in original SimDLL.dll"
            );
            None
        }
    }
}

/// 定位原版 SimDLL.dll：优先已加载模块；否则用 exe 目录拼
/// `OxygenNotIncluded_Data\Plugins\x86_64\SimDLL.dll` 完整路径 LoadLibrary。
/// 显式加载后，游戏 DllImport("SimDLL") 首次解析会命中该已加载模块
/// （按文件名匹配），入口已被接管。
#[cfg(windows)]
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
#[cfg(windows)]
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

    // 2026-08-10 修复：还原 old_protect（通常 PAGE_EXECUTE_READ），此前强制
    // PAGE_EXECUTE 会丢失 READ 属性（执行后立即跳转，实际影响有限，但语义错误）。
    let _ = VirtualProtect(target as *const c_void, JUMP_SIZE, old_protect, &mut old_protect);
    let _ = FlushInstructionCache(GetCurrentProcess(), Some(target), JUMP_SIZE);
    true
}

// ---------------------------------------------------------------------------
// Linux 实现：原版插件为 OxygenNotIncluded_Data/Plugins/x86_64/libSimDLL.so
// （Steam Linux 版命名，兼容 SimDLL.so）。查找用 dlopen(NOLOAD→显式)+dlsym，
// 改写用 mprotect（页对齐）；x86_64 自修改代码硬件保证指令缓存一致，
// 无需 FlushInstructionCache 等价物。
// ---------------------------------------------------------------------------
#[cfg(target_os = "linux")]
unsafe fn install_all() -> i32 {
    let mut installed = 0i32;
    for spec in hook_specs().iter() {
        if let Some(entry) = resolve_original_export(spec.name) {
            let ok = install_absolute_jump(entry, spec.target as *const c_void);
            trace_installed(spec.name, ok);
            if ok {
                installed += 1;
            }
        }
    }
    installed
}

/// 查找原版 SimDLL.so 中名为 `name` 的导出函数地址。
/// 优先 dlsym(RTLD_DEFAULT)（原版已被游戏 extern 绑定加载时直接命中；
/// 我们的 .so 由 C# 侧 dlopen 且不带 RTLD_GLOBAL，符号不在全局查找范围，
/// 不会误中自己的实现）；未加载则显式 dlopen 原版后重试。
#[cfg(target_os = "linux")]
unsafe fn resolve_original_export(name: &[u8]) -> Option<*mut c_void> {
    // C 字符串（去掉 Rust 侧的 NUL 尾巴，dlsym 自加）
    let mut c_name = name.to_vec();
    if c_name.last() == Some(&0) {
        c_name.pop();
    }
    c_name.push(0);

    let sym = libc::dlsym(libc::RTLD_DEFAULT, c_name.as_ptr() as *const libc::c_char);
    if !sym.is_null() {
        return Some(sym as *mut c_void);
    }

    let original = locate_original_module()?;
    let sym = libc::dlsym(original, c_name.as_ptr() as *const libc::c_char);
    if sym.is_null() {
        tracing::warn!(
            export = ?String::from_utf8_lossy(&name[..name.len().saturating_sub(1)]),
            "export not found in original SimDLL.so"
        );
        return None;
    }
    Some(sym as *mut c_void)
}

/// exe 路径（/proc/self/exe）→ 游戏目录 → 原版 SimDLL.so 候选路径。
/// Unity Linux 原生插件命名为 libSimDLL.so；兼容 SimDLL.so。
#[cfg(target_os = "linux")]
fn original_so_candidates() -> Vec<std::path::PathBuf> {
    let exe = match std::fs::read_link("/proc/self/exe") {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = ?e, "readlink /proc/self/exe failed");
            return Vec::new();
        }
    };
    let exe_dir = match exe.parent() {
        Some(d) => d.to_path_buf(),
        None => return Vec::new(),
    };
    let plugins = exe_dir
        .join("OxygenNotIncluded_Data")
        .join("Plugins")
        .join("x86_64");
    vec![plugins.join("libSimDLL.so"), plugins.join("SimDLL.so")]
}

/// 定位（必要时显式加载）原版 SimDLL.so。返回 dlopen 句柄。
/// 显式加载用 RTLD_GLOBAL：其导出进入全局符号表，与游戏后续 extern
/// 绑定行为一致（游戏 DllImport("SimDLL") 首次解析命中该已加载模块）。
#[cfg(target_os = "linux")]
unsafe fn locate_original_module() -> Option<*mut libc::c_void> {
    let mut candidates = original_so_candidates();
    // dlopen 绝对路径 NOLOAD：若游戏已加载（同名同 inode），直接返回现有句柄。
    for c in &candidates {
        let cpath = match std::ffi::CString::new(c.as_os_str().to_string_lossy().as_bytes()) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let h = libc::dlopen(
            cpath.as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_GLOBAL | libc::RTLD_NOLOAD,
        );
        if !h.is_null() {
            return Some(h);
        }
    }
    // 未加载 → 显式加载。
    candidates.retain(|c| c.exists());
    for c in &candidates {
        let cpath = match std::ffi::CString::new(c.as_os_str().to_string_lossy().as_bytes()) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let h = libc::dlopen(cpath.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL);
        if !h.is_null() {
            tracing::info!(path = ?c, "loaded original SimDLL.so");
            return Some(h);
        }
        tracing::error!(path = ?c, error = ?std::io::Error::last_os_error(), "dlopen failed");
    }
    tracing::error!("original SimDLL.so not found on disk");
    None
}

/// 在目标函数入口写 12 字节绝对跳转（同 Windows 版字节序列）。
/// Linux：mprotect 要求页对齐地址；写入窗口 RWX，完成后还原 R|X
/// （原版 .so 代码段为 r-xp）。x86_64 自修改代码硬件保证一致性，
/// 无需显式刷指令缓存。
#[cfg(target_os = "linux")]
unsafe fn install_absolute_jump(target: *mut c_void, dest: *const c_void) -> bool {
    const JUMP_SIZE: usize = 12;
    let page = libc::sysconf(libc::_SC_PAGESIZE) as usize;
    if page == 0 {
        return false;
    }
    let addr = (target as usize) & !(page - 1);
    let end = (target as usize + JUMP_SIZE).div_ceil(page) * page;
    let len = end - addr;

    if libc::mprotect(
        addr as *mut c_void,
        len,
        libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
    ) != 0
    {
        tracing::error!(error = ?std::io::Error::last_os_error(), "mprotect RWX failed");
        return false;
    }

    let bytes = target as *mut u8;
    *bytes.add(0) = 0x48; // mov rax, imm64
    *bytes.add(1) = 0xB8;
    std::ptr::write_unaligned(bytes.add(2) as *mut u64, dest as u64);
    *bytes.add(10) = 0xFF; // jmp rax
    *bytes.add(11) = 0xE0;

    // 还原代码页为可读可执行（原版 .so 代码段 r-xp；Linux 无法查询单页旧保护，
    // 按段属性还原是 detour 例行做法）。
    if libc::mprotect(addr as *mut c_void, len, libc::PROT_READ | libc::PROT_EXEC) != 0 {
        tracing::error!(error = ?std::io::Error::last_os_error(), "mprotect restore RX failed");
        // 页仍为 RWX：功能正确（跳转已生效），仅保护属性宽于原版——不判失败。
    }
    true
}
