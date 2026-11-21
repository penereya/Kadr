#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[allow(dead_code)]
mod theme;

use eframe::egui::{self, Align, Layout, Margin, Rounding, Stroke, Vec2};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

static KADR_EXE: &[u8] = include_bytes!("../target/release/kadr.exe");
static KADR_ICO: &[u8] = include_bytes!("../assets/kadr.ico");

fn load_app_icon() -> egui::IconData {
    let bytes = include_bytes!("../assets/app_icon.png");
    let img = image::load_from_memory(bytes)
        .expect("embedded app_icon.png must be valid PNG")
        .into_rgba8();
    let (width, height) = img.dimensions();
    egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    }
}

fn default_install_dir() -> PathBuf {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("Programs").join("Kadr");
    }
    PathBuf::from(r"C:\Kadr")
}

fn desktop_shortcut_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Desktop").join("Kadr.lnk"))
}

fn start_menu_shortcut_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| {
        PathBuf::from(p)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Kadr.lnk")
    })
}

fn escape_ps_single(s: &str) -> String {
    s.replace('\'', "''")
}

fn run_ps(script: &str) -> Result<(), String> {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Не удалось запустить PowerShell: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            "Ошибка выполнения PowerShell".to_string()
        } else {
            stderr
        })
    }
}

fn create_shortcut(lnk_path: &Path, target: &Path, icon: &Path, work_dir: &Path) -> Result<(), String> {
    if let Some(parent) = lnk_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Не удалось создать папку для ярлыка: {}", e))?;
    }
    let script = format!(
        "$ws = New-Object -ComObject WScript.Shell; \
         $s = $ws.CreateShortcut('{}'); \
         $s.TargetPath = '{}'; \
         $s.WorkingDirectory = '{}'; \
         $s.IconLocation = '{}'; \
         $s.Description = 'Kadr — Медиаредактор'; \
         $s.Save()",
        escape_ps_single(&lnk_path.to_string_lossy()),
        escape_ps_single(&target.to_string_lossy()),
        escape_ps_single(&work_dir.to_string_lossy()),
        escape_ps_single(&icon.to_string_lossy()),
    );
    run_ps(&script)
}

fn register_uninstall_entry(install_dir: &Path, ico_path: &Path, uninstall_exe: &Path) -> Result<(), String> {
    let size_kb = (KADR_EXE.len() / 1024).max(1);
    let key = r"HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Kadr";
    let uninst_cmd = format!("\"{}\" --uninstall", uninstall_exe.to_string_lossy());
    let script = format!(
        "New-Item -Path '{}' -Force | Out-Null; \
         Set-ItemProperty -Path '{}' -Name 'DisplayName' -Value 'Kadr — Медиаредактор'; \
         Set-ItemProperty -Path '{}' -Name 'DisplayVersion' -Value '{}'; \
         Set-ItemProperty -Path '{}' -Name 'Publisher' -Value 'penereya'; \
         Set-ItemProperty -Path '{}' -Name 'DisplayIcon' -Value '{}'; \
         Set-ItemProperty -Path '{}' -Name 'InstallLocation' -Value '{}'; \
         Set-ItemProperty -Path '{}' -Name 'UninstallString' -Value '{}'; \
         Set-ItemProperty -Path '{}' -Name 'EstimatedSize' -Type DWord -Value {}; \
         Set-ItemProperty -Path '{}' -Name 'NoModify' -Type DWord -Value 1; \
         Set-ItemProperty -Path '{}' -Name 'NoRepair' -Type DWord -Value 1",
        key,
        key,
        key,
        APP_VERSION,
        key,
        key,
        escape_ps_single(&ico_path.to_string_lossy()),
        key,
        escape_ps_single(&install_dir.to_string_lossy()),
        key,
        escape_ps_single(&uninst_cmd),
        key,
        size_kb,
        key,
        key,
    );
    run_ps(&script)
}

fn perform_install(
    install_dir: &Path,
    make_desktop_lnk: bool,
    make_start_lnk: bool,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(install_dir)
        .map_err(|e| format!("Не удалось создать папку {}: {}", install_dir.display(), e))?;

    let exe_path = install_dir.join("Kadr.exe");
    std::fs::write(&exe_path, KADR_EXE)
        .map_err(|e| format!("Не удалось записать Kadr.exe: {}", e))?;

    let ico_path = install_dir.join("kadr.ico");
    std::fs::write(&ico_path, KADR_ICO)
        .map_err(|e| format!("Не удалось записать kadr.ico: {}", e))?;

    let uninstall_path = install_dir.join("Uninstall.exe");
    if let Ok(self_exe) = std::env::current_exe() {
        if self_exe != uninstall_path {
            let _ = std::fs::copy(&self_exe, &uninstall_path);
        }
    }

    if make_desktop_lnk {
        if let Some(lnk) = desktop_shortcut_path() {
            create_shortcut(&lnk, &exe_path, &ico_path, install_dir)?;
        }
    }
    if make_start_lnk {
        if let Some(lnk) = start_menu_shortcut_path() {
            create_shortcut(&lnk, &exe_path, &ico_path, install_dir)?;
        }
    }

    let _ = register_uninstall_entry(install_dir, &ico_path, &uninstall_path);
    let _ = register_open_with(&exe_path, &ico_path);
    Ok(exe_path)
}

fn register_open_with(exe_path: &Path, ico_path: &Path) -> Result<(), String> {
    let base = r"HKCU:\Software\Classes\Applications\Kadr.exe";
    let prog_id = r"HKCU:\Software\Classes\Kadr.Media";
    let cap = r"HKCU:\Software\Kadr\Capabilities";
    let cmd = format!("\"{}\" \"%1\"", exe_path.to_string_lossy());
    let ico_escaped = escape_ps_single(&ico_path.to_string_lossy());
    let cmd_escaped = escape_ps_single(&cmd);

    let exts = [
        ".png", ".jpg", ".jpeg", ".webp", ".bmp", ".gif",
        ".mp4", ".mkv", ".avi", ".mov", ".webm",
        ".mp3", ".wav", ".ogg", ".flac", ".m4a",
    ];
    let mut ext_cmds = String::new();
    for ext in &exts {
        ext_cmds.push_str(&format!(
            "Set-ItemProperty -Path '{}\\SupportedTypes' -Name '{}' -Value ''; \
             New-Item -Path 'HKCU:\\Software\\Classes\\{}\\OpenWithProgids' -Force | Out-Null; \
             Set-ItemProperty -Path 'HKCU:\\Software\\Classes\\{}\\OpenWithProgids' -Name 'Kadr.Media' -Value ''; \
             Set-ItemProperty -Path '{}\\FileAssociations' -Name '{}' -Value 'Kadr.Media'; ",
            base, ext, ext, ext, cap, ext
        ));
    }

    for assoc in &["video", "image", "audio", ".mp4", ".mkv", ".avi", ".mov", ".webm", ".png", ".jpg", ".jpeg", ".webp", ".mp3", ".wav", ".flac"] {
        ext_cmds.push_str(&format!(
            "New-Item -Path 'HKCU:\\Software\\Classes\\SystemFileAssociations\\{assoc}\\shell\\EditWithKadr\\command' -Force | Out-Null; \
             Set-ItemProperty -Path 'HKCU:\\Software\\Classes\\SystemFileAssociations\\{assoc}\\shell\\EditWithKadr' -Name '(default)' -Value 'Редактировать с помощью Kadr'; \
             Set-ItemProperty -Path 'HKCU:\\Software\\Classes\\SystemFileAssociations\\{assoc}\\shell\\EditWithKadr' -Name 'Icon' -Value '{ico}'; \
             Set-ItemProperty -Path 'HKCU:\\Software\\Classes\\SystemFileAssociations\\{assoc}\\shell\\EditWithKadr\\command' -Name '(default)' -Value '{cmd}'; ",
            assoc = assoc,
            ico = ico_escaped,
            cmd = cmd_escaped,
        ));
    }

    let script = format!(
        "New-Item -Path '{base}' -Force | Out-Null; \
         Set-ItemProperty -Path '{base}' -Name 'FriendlyAppName' -Value 'Kadr — Медиаредактор'; \
         New-Item -Path '{base}\\DefaultIcon' -Force | Out-Null; \
         Set-ItemProperty -Path '{base}\\DefaultIcon' -Name '(default)' -Value '{ico}'; \
         New-Item -Path '{base}\\shell\\open\\command' -Force | Out-Null; \
         Set-ItemProperty -Path '{base}\\shell\\open\\command' -Name '(default)' -Value '{cmd}'; \
         New-Item -Path '{base}\\SupportedTypes' -Force | Out-Null; \
         New-Item -Path '{prog}' -Force | Out-Null; \
         Set-ItemProperty -Path '{prog}' -Name '(default)' -Value 'Kadr — Медиафайл'; \
         New-Item -Path '{prog}\\DefaultIcon' -Force | Out-Null; \
         Set-ItemProperty -Path '{prog}\\DefaultIcon' -Name '(default)' -Value '{ico}'; \
         New-Item -Path '{prog}\\shell\\open\\command' -Force | Out-Null; \
         Set-ItemProperty -Path '{prog}\\shell\\open\\command' -Name '(default)' -Value '{cmd}'; \
         New-Item -Path '{cap}\\FileAssociations' -Force | Out-Null; \
         Set-ItemProperty -Path '{cap}' -Name 'ApplicationName' -Value 'Kadr — Медиаредактор'; \
         Set-ItemProperty -Path '{cap}' -Name 'ApplicationDescription' -Value 'Простое и лёгкое приложение с базовыми функциями редактора медиа'; \
         New-Item -Path 'HKCU:\\Software\\RegisteredApplications' -Force | Out-Null; \
         Set-ItemProperty -Path 'HKCU:\\Software\\RegisteredApplications' -Name 'Kadr' -Value 'Software\\Kadr\\Capabilities'; \
         {ext_cmds}",
        base = base,
        prog = prog_id,
        cap = cap,
        ico = ico_escaped,
        cmd = cmd_escaped,
        ext_cmds = ext_cmds,
    );
    run_ps(&script)
}

fn perform_uninstall() -> Result<(), String> {
    if let Some(lnk) = desktop_shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }
    if let Some(lnk) = start_menu_shortcut_path() {
        let _ = std::fs::remove_file(lnk);
    }
    let _ = run_ps(
        r#"foreach ($a in @('video','image','audio','.mp4','.mkv','.avi','.mov','.webm','.png','.jpg','.jpeg','.webp','.mp3','.wav','.flac')) { Remove-Item -Path "HKCU:\Software\Classes\SystemFileAssociations\$a\shell\EditWithKadr" -Recurse -Force -ErrorAction SilentlyContinue }; Remove-Item -Path 'HKCU:\Software\Classes\Applications\Kadr.exe' -Recurse -Force -ErrorAction SilentlyContinue; Remove-Item -Path 'HKCU:\Software\Classes\Kadr.Media' -Recurse -Force -ErrorAction SilentlyContinue; Remove-Item -Path 'HKCU:\Software\Kadr' -Recurse -Force -ErrorAction SilentlyContinue; Remove-ItemProperty -Path 'HKCU:\Software\RegisteredApplications' -Name 'Kadr' -ErrorAction SilentlyContinue; Remove-Item -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Kadr' -Recurse -Force -ErrorAction SilentlyContinue"#,
    );

    let dir = default_install_dir();
    let _ = std::fs::remove_file(dir.join("Kadr.exe"));
    let _ = std::fs::remove_file(dir.join("kadr.ico"));

    if dir.exists() {
        let cmd_str = format!("ping 127.0.0.1 -n 2 > nul & rd /s /q \"{}\"", dir.display());
        let _ = Command::new("cmd")
            .args(["/C", &cmd_str])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
enum InstallState {
    Ready,
    Done(PathBuf),
    Uninstalled,
    Error(String),
}

struct InstallerApp {
    install_dir: PathBuf,
    desktop_shortcut: bool,
    start_menu_shortcut: bool,
    state: InstallState,
    logo_texture: Option<egui::TextureHandle>,
}

impl InstallerApp {
    fn new(cc: &eframe::CreationContext<'_>, uninstall_mode: bool) -> Self {
        theme::setup_fonts(&cc.egui_ctx);
        theme::apply_graphite_theme(&cc.egui_ctx);

        let logo_texture = image::load_from_memory(include_bytes!("../assets/logo.png"))
            .ok()
            .map(|img| {
                let rgba = img.into_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color_img = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                cc.egui_ctx
                    .load_texture("kadr_setup_logo", color_img, egui::TextureOptions::LINEAR)
            });

        let state = if uninstall_mode {
            match perform_uninstall() {
                Ok(()) => InstallState::Uninstalled,
                Err(e) => InstallState::Error(e),
            }
        } else {
            InstallState::Ready
        };

        Self {
            install_dir: default_install_dir(),
            desktop_shortcut: true,
            start_menu_shortcut: true,
            state,
            logo_texture,
        }
    }
}

fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let w = ui.available_width().max(120.0);
    let btn = egui::Button::new(
        egui::RichText::new(text)
            .size(14.5)
            .color(egui::Color32::WHITE)
            .strong(),
    )
    .fill(theme::ACCENT)
    .stroke(Stroke::NONE)
    .rounding(Rounding::same(10.0))
    .min_size(Vec2::new(w, 42.0));
    ui.add_sized(Vec2::new(w, 42.0), btn)
}

impl eframe::App for InstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(theme::BG_BASE)
                    .inner_margin(Margin::same(24.0)),
            )
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    // Заголовок с логотипом
                    ui.horizontal(|ui| {
                        if let Some(tex) = &self.logo_texture {
                            ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(42.0)));
                            ui.add_space(12.0);
                        }
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Установка Kadr v{}", APP_VERSION))
                                    .size(20.0)
                                    .color(theme::TEXT_PRIMARY)
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(
                                    "Простое и лёгкое приложение для фото, видео и аудио",
                                )
                                .size(13.0)
                                .color(theme::TEXT_SECONDARY),
                            );
                        });
                    });

                    ui.add_space(20.0);

                    match &self.state {
                        InstallState::Ready => {
                            egui::Frame::none()
                                .fill(theme::BG_SURFACE)
                                .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                                .rounding(Rounding::same(12.0))
                                .inner_margin(Margin::same(16.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        egui::RichText::new("Папка установки")
                                            .size(12.5)
                                            .color(theme::TEXT_SECONDARY)
                                            .strong(),
                                    );
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(
                                                self.install_dir.to_string_lossy().to_string(),
                                            )
                                            .size(13.0)
                                            .color(theme::TEXT_PRIMARY),
                                        );
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if theme::subtle_button(ui, "Изменить...").clicked()
                                                {
                                                    if let Some(dir) = rfd::FileDialog::new()
                                                        .set_directory(&self.install_dir)
                                                        .pick_folder()
                                                    {
                                                        self.install_dir = dir.join("Kadr");
                                                    }
                                                }
                                            },
                                        );
                                    });

                                    ui.add_space(12.0);
                                    ui.checkbox(
                                        &mut self.desktop_shortcut,
                                        "Создать ярлык на рабочем столе",
                                    );
                                    ui.add_space(4.0);
                                    ui.checkbox(
                                        &mut self.start_menu_shortcut,
                                        "Добавить в меню «Пуск»",
                                    );
                                });

                            ui.add_space(18.0);
                            if primary_button(ui, "Установить Kadr").clicked() {
                                match perform_install(
                                    &self.install_dir,
                                    self.desktop_shortcut,
                                    self.start_menu_shortcut,
                                ) {
                                    Ok(exe) => self.state = InstallState::Done(exe),
                                    Err(err) => self.state = InstallState::Error(err),
                                }
                            }
                        }
                        InstallState::Done(exe_path) => {
                            let exe_clone = exe_path.clone();
                            egui::Frame::none()
                                .fill(theme::BG_SURFACE)
                                .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                                .rounding(Rounding::same(12.0))
                                .inner_margin(Margin::same(18.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        egui::RichText::new("✓ Kadr успешно установлен!")
                                            .size(16.0)
                                            .color(theme::SUCCESS)
                                            .strong(),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Путь: {}",
                                            self.install_dir.display()
                                        ))
                                        .size(12.5)
                                        .color(theme::TEXT_SECONDARY),
                                    );
                                });

                            ui.add_space(18.0);
                            if primary_button(ui, "Запустить Kadr").clicked() {
                                let _ = Command::new(&exe_clone).spawn();
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            ui.add_space(8.0);
                            if theme::subtle_button(ui, "Закрыть").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        InstallState::Uninstalled => {
                            egui::Frame::none()
                                .fill(theme::BG_SURFACE)
                                .stroke(Stroke::new(1.0_f32, theme::BORDER_SUBTLE))
                                .rounding(Rounding::same(12.0))
                                .inner_margin(Margin::same(18.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        egui::RichText::new("Kadr удалён с компьютера")
                                            .size(16.0)
                                            .color(theme::TEXT_PRIMARY)
                                            .strong(),
                                    );
                                });
                            ui.add_space(20.0);
                            if theme::subtle_button(ui, "Закрыть").clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        InstallState::Error(msg) => {
                            let msg = msg.clone();
                            egui::Frame::none()
                                .fill(theme::BG_SURFACE)
                                .stroke(Stroke::new(1.0_f32, theme::ERROR))
                                .rounding(Rounding::same(12.0))
                                .inner_margin(Margin::same(16.0))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(
                                        egui::RichText::new("Ошибка установки")
                                            .size(15.0)
                                            .color(theme::ERROR)
                                            .strong(),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        egui::RichText::new(msg)
                                            .size(12.5)
                                            .color(theme::TEXT_PRIMARY),
                                    );
                                });
                            ui.add_space(16.0);
                            if theme::subtle_button(ui, "Попробовать снова").clicked() {
                                self.state = InstallState::Ready;
                            }
                        }
                    }
                });
            });
    }
}

fn main() -> eframe::Result<()> {
    let uninstall_mode = std::env::args().any(|a| a == "--uninstall");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Установка Kadr")
            .with_inner_size([520.0, 370.0])
            .with_resizable(false)
            .with_icon(load_app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "KadrSetup",
        options,
        Box::new(move |cc| Ok(Box::new(InstallerApp::new(cc, uninstall_mode)))),
    )
}
