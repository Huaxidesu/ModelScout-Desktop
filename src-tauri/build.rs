// Tauri 官方标准构建脚本。
// 它会读取 tauri.conf.json、生成权限 schema（gen/schemas）、
// 并为 Windows 目标嵌入应用清单与资源。
fn main() {
    tauri_build::build()
}
