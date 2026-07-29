fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows-msvc") {
        panic!(
            "simDLL_RS 仅支持 x86_64-pc-windows-msvc 目标，当前目标: {target}. \
             使用: cargo build --release --target x86_64-pc-windows-msvc"
        );
    }
    println!("cargo:rerun-if-changed=build.rs");
}
