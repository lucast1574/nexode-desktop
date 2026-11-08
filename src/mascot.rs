//! Nexode logo, rendered by egui's GPU image pipeline. No web runtime or animation timer.
use eframe::egui::{self, Image, Rect, Response, Sense, Vec2};
use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const FRAME_INTERVAL: Duration = Duration::from_nanos(33_333_334);
const PREFERENCE_INTERVAL: Duration = Duration::from_secs(30);
const COMPLETION_SECONDS: f64 = 0.85;

#[derive(Clone, Copy, Default)]
struct MotionState {
    was_working: bool,
    completed_at: Option<f64>,
}

#[derive(Clone, Copy)]
struct Pose {
    scale: f32,
    y: f32,
    rotation: f32,
}

/// `animate` must be false while the window is hidden/minimized (or unfocused,
/// if the host chooses). Offscreen images never schedule frames. Reduced-motion
/// preferences override `animate` automatically; there is no animation toggle.
pub fn show(ui: &mut egui::Ui, size: f32, working: bool, animate: bool) -> Response {
    let size = if size.is_finite() {
        size.clamp(8.0, 1024.0)
    } else {
        64.0
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let now = ui.input(|input| input.time);
    let id = response.id.with("nexo-motion");
    let state = ui.ctx().data_mut(|data| {
        let state = data.get_temp_mut_or_default::<MotionState>(id);
        if state.was_working && !working {
            state.completed_at = Some(now);
        }
        state.was_working = working;
        if working
            || state
                .completed_at
                .is_some_and(|at| now - at > COMPLETION_SECONDS)
        {
            state.completed_at = None;
        }
        *state
    });
    let moving = animate && !reduced_motion(ui.ctx());
    let pose = if moving {
        ui.ctx().request_repaint_after(FRAME_INTERVAL);
        pose(now, working, state.completed_at.map(|at| now - at))
    } else {
        Pose {
            scale: 1.0,
            y: 0.0,
            rotation: 0.0,
        }
    };
    // Reserve breathing room so transforms never overlap adjacent text or clip.
    let image_rect = Rect::from_center_size(
        rect.center() + egui::vec2(0.0, size * pose.y),
        Vec2::splat(size * 0.82 * pose.scale),
    );
    Image::new(egui::include_image!("../assets/nexode.svg"))
        .rotate(pose.rotation, Vec2::splat(0.5))
        .paint_at(ui, image_rect);
    response.on_hover_text(if working {
        "Nexode está trabajando"
    } else {
        "Nexode Orbit"
    })
}

fn pose(time: f64, working: bool, completion_age: Option<f64>) -> Pose {
    let time = if time.is_finite() { time } else { 0.0 };
    let period = if working { 1.6 } else { 4.8 };
    let phase = ((time % period) * std::f64::consts::TAU / period) as f32;
    let mut result = Pose {
        scale: 1.0 + phase.sin() * 0.015,
        y: phase.cos() * if working { 0.012 } else { 0.016 },
        rotation: phase.sin() * if working { 0.055 } else { 0.022 },
    };
    if let Some(age) =
        completion_age.filter(|age| age.is_finite() && *age >= 0.0 && *age < COMPLETION_SECONDS)
    {
        let progress = (age / COMPLETION_SECONDS) as f32;
        let bounce = (progress * std::f32::consts::PI).sin();
        result.y -= bounce * 0.035;
        result.scale += bounce * 0.015;
        result.rotation += (progress * std::f32::consts::TAU).sin() * 0.03;
    }
    result
}

struct MotionPreference {
    reduced: AtomicBool,
    checking: AtomicBool,
    checked_at: Mutex<Option<Instant>>,
}

pub(crate) fn reduced_motion(ctx: &egui::Context) -> bool {
    static PREFERENCE: OnceLock<MotionPreference> = OnceLock::new();
    let preference = PREFERENCE.get_or_init(|| MotionPreference {
        // Unknown accessibility preference means static artwork, never guess.
        reduced: AtomicBool::new(true),
        checking: AtomicBool::new(false),
        checked_at: Mutex::new(None),
    });
    let due = preference
        .checked_at
        .lock()
        .map(|at| at.is_none_or(|at| at.elapsed() >= PREFERENCE_INTERVAL))
        .unwrap_or(false);
    if due && !preference.checking.swap(true, Ordering::AcqRel) {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let forced = std::env::var("ORBIT_REDUCED_MOTION")
                .ok()
                .is_some_and(|value| {
                    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
                });
            preference.reduced.store(
                forced || os_reduced_motion().unwrap_or(true),
                Ordering::Release,
            );
            if let Ok(mut at) = preference.checked_at.lock() {
                *at = Some(Instant::now());
            }
            preference.checking.store(false, Ordering::Release);
            ctx.request_repaint();
        });
    }
    preference.reduced.load(Ordering::Acquire)
}

#[cfg(windows)]
fn os_reduced_motion() -> Option<bool> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(
            action: u32,
            param: u32,
            value: *mut core::ffi::c_void,
            flags: u32,
        ) -> i32;
    }
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut enabled = 0_i32;
    // Read-only Win32 accessibility preference; pvParam points to a BOOL.
    let succeeded = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    (succeeded != 0).then_some(enabled == 0)
}

#[cfg(target_os = "macos")]
fn os_reduced_motion() -> Option<bool> {
    Some(objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
}

#[cfg(target_os = "linux")]
fn os_reduced_motion() -> Option<bool> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if desktop.contains("kde") {
        for program in ["kreadconfig6", "kreadconfig5"] {
            if let Some(value) = preference_command(
                program,
                &[
                    "--file",
                    "kdeglobals",
                    "--group",
                    "KDE",
                    "--key",
                    "AnimationDurationFactor",
                    "--default",
                    "1",
                ],
            ) && let Some(reduced) = kde_reduced_motion(&value)
            {
                return Some(reduced);
            }
        }
    }
    preference_command(
        "gsettings",
        &["get", "org.gnome.desktop.interface", "enable-animations"],
    )
    .and_then(|value| gnome_reduced_motion(&value))
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn os_reduced_motion() -> Option<bool> {
    None
}

#[cfg(any(target_os = "linux", test))]
fn gnome_reduced_motion(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(false),
        "false" => Some(true),
        _ => None,
    }
}
#[cfg(any(target_os = "linux", test))]
fn kde_reduced_motion(value: &str) -> Option<bool> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value == 0.0)
}

#[cfg(target_os = "linux")]
fn preference_command(program: &str, args: &[&str]) -> Option<String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut value = String::new();
                child
                    .stdout
                    .take()?
                    .take(128)
                    .read_to_string(&mut value)
                    .ok()?;
                return Some(value);
            }
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_stays_inside_reserved_widget_and_schedules_at_most_thirty_fps() {
        assert!(FRAME_INTERVAL.as_secs_f64() >= 1.0 / 30.0);
        for working in [false, true] {
            for tick in 0..2000 {
                for age in [
                    None,
                    Some(0.0),
                    Some(0.2),
                    Some(0.425),
                    Some(0.84),
                    Some(1.0),
                ] {
                    let p = pose(f64::from(tick) * 0.031, working, age);
                    let rotated_half_extent =
                        0.41 * p.scale * (p.rotation.cos().abs() + p.rotation.sin().abs());
                    assert!(rotated_half_extent + p.y.abs() < 0.5);
                    assert!((0.98..=1.031).contains(&p.scale));
                }
            }
        }
    }
    #[test]
    fn system_preferences_distinguish_reduced_motion_from_unknown() {
        assert_eq!(gnome_reduced_motion("false\n"), Some(true));
        assert_eq!(gnome_reduced_motion("true"), Some(false));
        assert_eq!(gnome_reduced_motion("unavailable"), None);
        assert_eq!(kde_reduced_motion("0"), Some(true));
        assert_eq!(kde_reduced_motion("0.5"), Some(false));
        assert_eq!(kde_reduced_motion("NaN"), None);
        assert_eq!(kde_reduced_motion("-1"), None);
    }
}
