use tauri::{Manager, Emitter};
use tauri_plugin_shell::ShellExt;
use tauri_plugin_shell::process::CommandEvent;
use std::env;
use std::collections::HashMap;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let exe_path = env::current_exe().expect("无法获取当前可执行文件路径");
            let sidecar_dir = exe_path.parent().expect("无法获取可执行文件的父目录");

            // 收集所有系统环境变量
            let mut all_env_vars = HashMap::new();
            for (key, value) in env::vars() {
                all_env_vars.insert(key, value);
            }

            // 增强 PATH 环境变量，添加常见的二进制路径
            let current_path = all_env_vars.get("PATH").cloned().unwrap_or_default();
            let additional_paths = get_additional_binary_paths();
            let enhanced_path = enhance_path_variable(&current_path, &additional_paths);
            all_env_vars.insert("PATH".to_string(), enhanced_path);

            // 打印一些关键环境变量用于调试
            println!("Enhanced PATH: {}", all_env_vars.get("PATH").unwrap_or(&"Not found".to_string()));
            println!("Total environment variables: {}", all_env_vars.len());

            // 检查 ffmpeg 是否在 PATH 中可用
            check_ffmpeg_availability();

            // 启动 Python 后端侧车
            let mut sidecar_command = app.shell().sidecar("BiliNoteBackend").unwrap();

            // 设置所有环境变量到 sidecar
            for (key, value) in &all_env_vars {
                sidecar_command = sidecar_command.env(key, value);
            }

            let (mut rx, _child) = sidecar_command
                .current_dir(sidecar_dir)
                .spawn()
                .expect("Failed to spawn sidecar");

            // 获取主窗口句柄用于发送事件
            let window = app.get_webview_window("main").unwrap();

            tauri::async_runtime::spawn(async move {
                // 读取诸如 stdout 之类的事件
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(line) => {
                            let output = String::from_utf8_lossy(&line);
                            println!("Backend stdout: {}", output);

                            // 发送到前端
                            window
                                .emit("backend-message", Some(format!("'{}'", output)))
                                .expect("failed to emit event");
                        }
                        CommandEvent::Stderr(line) => {
                            let error = String::from_utf8_lossy(&line);
                            eprintln!("Backend stderr: {}", error);

                            window
                                .emit("backend-error", Some(format!("'{}'", error)))
                                .expect("failed to emit event");
                        }
                        CommandEvent::Terminated(payload) => {
                            println!("Backend terminated with code: {:?}", payload.code);
                            window
                                .emit("backend-terminated", Some(payload.code))
                                .expect("failed to emit event");
                            break;
                        }
                        _ => {
                            println!("Backend event: {:?}", event);
                        }
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_system_env_vars,
            find_executable_path,
            run_command_with_env,
            test_ffmpeg_access
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// 获取额外的二进制路径
fn get_additional_binary_paths() -> Vec<String> {
    if cfg!(target_os = "windows") {
        vec![
            "C:\\ffmpeg\\bin".to_string(),
            "C:\\Program Files\\ffmpeg\\bin".to_string(),
            "C:\\Program Files (x86)\\ffmpeg\\bin".to_string(),
            "C:\\tools\\ffmpeg\\bin".to_string(),
            "C:\\ProgramData\\chocolatey\\bin".to_string(),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            "/usr/local/bin".to_string(),
            "/opt/homebrew/bin".to_string(),
            "/usr/bin".to_string(),
            "/bin".to_string(),
            "/opt/local/bin".to_string(), // MacPorts
        ]
    } else {
        vec![
            "/usr/local/bin".to_string(),
            "/usr/bin".to_string(),
            "/bin".to_string(),
            "/snap/bin".to_string(),
            "/opt/bin".to_string(),
            "/usr/local/sbin".to_string(),
        ]
    }
}

// 增强 PATH 环境变量
fn enhance_path_variable(current_path: &str, additional_paths: &[String]) -> String {
    let path_separator = if cfg!(target_os = "windows") { ";" } else { ":" };

    let mut paths: Vec<String> = additional_paths.to_vec();

    // 添加当前 PATH
    if !current_path.is_empty() {
        paths.push(current_path.to_string());
    }

    paths.join(path_separator)
}

// 检查 ffmpeg 可用性
fn check_ffmpeg_availability() {
    use std::process::Command;

    match Command::new("ffmpeg").arg("-version").output() {
        Ok(output) => {
            if output.status.success() {
                println!("✓ FFmpeg is available in PATH");
                let version_info = String::from_utf8_lossy(&output.stdout);
                let first_line = version_info.lines().next().unwrap_or("Unknown version");
                println!("FFmpeg version: {}", first_line);
            } else {
                println!("✗ FFmpeg found but returned error");
            }
        }
        Err(e) => {
            println!("✗ FFmpeg not found in PATH: {}", e);

            // 尝试在常见路径中查找
            let common_paths = get_additional_binary_paths();
            for path in common_paths {
                let ffmpeg_path = if cfg!(target_os = "windows") {
                    format!("{}\\ffmpeg.exe", path)
                } else {
                    format!("{}/ffmpeg", path)
                };

                if std::path::Path::new(&ffmpeg_path).exists() {
                    println!("✓ Found FFmpeg at: {}", ffmpeg_path);
                    return;
                }
            }
            println!("✗ FFmpeg not found in common installation paths");
        }
    }
}

// Tauri 命令：获取系统环境变量
#[tauri::command]
fn get_system_env_vars() -> HashMap<String, String> {
    env::vars().collect()
}

// Tauri 命令：查找可执行文件路径
#[tauri::command]
fn find_executable_path(executable_name: String) -> Option<String> {
    use std::process::Command;

    // 首先尝试直接执行
    if Command::new(&executable_name).arg("--version").output().is_ok() {
        return Some(executable_name);
    }

    // 使用 which/where 命令查找
    let which_cmd = if cfg!(target_os = "windows") { "where" } else { "which" };

    if let Ok(output) = Command::new(which_cmd).arg(&executable_name).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    // 在常见路径中搜索
    let common_paths = get_additional_binary_paths();
    for base_path in common_paths {
        let executable_path = if cfg!(target_os = "windows") {
            format!("{}\\{}.exe", base_path, executable_name)
        } else {
            format!("{}/{}", base_path, executable_name)
        };

        if std::path::Path::new(&executable_path).exists() {
            return Some(executable_path);
        }
    }

    None
}

// Tauri 命令：使用完整环境变量运行命令
#[tauri::command]
async fn run_command_with_env(
    program: String,
    args: Vec<String>
) -> Result<String, String> {
    use std::process::Command;

    let mut cmd = Command::new(&program);
    cmd.args(&args);

    // 设置所有环境变量
    for (key, value) in env::vars() {
        cmd.env(key, value);
    }

    // 增强 PATH
    let current_path = env::var("PATH").unwrap_or_default();
    let additional_paths = get_additional_binary_paths();
    let enhanced_path = enhance_path_variable(&current_path, &additional_paths);
    cmd.env("PATH", enhanced_path);

    match cmd.output() {
        Ok(output) => {
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).to_string())
            }
        }
        Err(e) => Err(format!("Failed to execute {}: {}", program, e))
    }
}

// Tauri 命令：测试 ffmpeg 访问
#[tauri::command]
async fn test_ffmpeg_access() -> Result<String, String> {
    run_command_with_env("ffmpeg".to_string(), vec!["-version".to_string()]).await
}

// 可选：添加一个函数来动态更新 sidecar 的环境变量
#[tauri::command]
async fn update_sidecar_environment(
    app_handle: tauri::AppHandle,
    additional_env_vars: HashMap<String, String>
) -> Result<(), String> {
    // 这个函数可以用来在运行时更新环境变量
    // 注意：这需要重启 sidecar 才能生效

    for (key, value) in additional_env_vars {
        env::set_var(key, value);
    }

    Ok(())
}