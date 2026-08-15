fn main() {
    // tauri-build 默认只监听 tauri.conf.json，不会因图标文件变化而重新嵌入
    // exe 图标资源。显式声明 icons 目录，保证换图标后构建脚本必然重跑。
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
