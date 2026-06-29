//! In-canvas diagnostic drawer + the persistent UI surfaces (wall clock,
//! build watermark). The drawer is hidden by default; pressing
//! `` ` `` or `\` toggles it.
//!
//! Every observable surface lives here: FPS, ErrorLog tail, network
//! stats (self peer-id + connected peer count), and a keys legend.
//! The wall clock + build watermark stay on screen always.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::build_info;
use crate::observability::{ErrorLog, Severity};

#[derive(Component)]
pub struct ClockText;

#[derive(Component)]
pub struct FpsText;

#[derive(Component)]
pub struct ErrorListText;

#[derive(Component)]
pub struct LogDrawer;

#[cfg(target_arch = "wasm32")]
#[derive(Component)]
pub struct NetStatsText;

/// Spawn the wall-clock top-right, build watermark bottom-right, the
/// drawer (hidden by default), and the inventory placeholder strip
/// along the bottom. Called from `crate::run`'s Startup.
pub fn setup_drawer(mut commands: Commands) {
    commands.spawn((
        Text::new("HH:MM:SS.mmm  GMT±HHMM"),
        ClockText,
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(Color::srgb(0.59, 0.59, 0.59)),
        Node {
            position_type: PositionType::Absolute,
            top: px(6),
            right: px(6),
            ..default()
        },
    ));

    commands.spawn((
        Text::new(format!("rave · {} · {}", build_info::COMMIT, build_info::BUILT_AT)),
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(Color::srgb(0.55, 0.55, 0.55)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(6),
            right: px(6),
            ..default()
        },
    ));

    commands
        .spawn((
            LogDrawer,
            Node {
                position_type: PositionType::Absolute,
                top: px(0),
                left: px(0),
                width: Val::Percent(100.0),
                height: Val::Percent(40.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(12)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
            Visibility::Hidden,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(
                    "keys: WASD move · `/\\ toggle · P screenshot",
                ),
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.55, 0.55, 0.55)),
            ));
            #[cfg(target_arch = "wasm32")]
            parent.spawn((
                Text::new("net: …"),
                NetStatsText,
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.65, 0.85, 0.65)),
            ));
            parent.spawn((
                Text::new("FPS"),
                FpsText,
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.85)),
            ));
            parent.spawn((
                Text::new(""),
                ErrorListText,
                TextFont {
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.4, 0.4)),
            ));
        });

    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: px(20),
            left: Val::Percent(50.0),
            margin: UiRect {
                left: px(-180),
                ..default()
            },
            width: px(360),
            height: px(40),
            ..default()
        })
        .with_children(|parent| {
            for _ in 0..9 {
                parent.spawn((
                    Node {
                        width: px(36),
                        height: px(36),
                        margin: UiRect::all(px(2)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.12, 0.12, 0.12)),
                ));
            }
        });
}

pub fn update_fps(
    diagnostics: Res<DiagnosticsStore>,
    mut texts: Query<&mut Text, With<FpsText>>,
    mut log: ResMut<ErrorLog>,
) {
    let Some(diag) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS) else {
        log.emit(
            Severity::Error,
            "FrameTimeDiagnosticsPlugin::FPS not registered",
        );
        return;
    };
    let display = match diag.smoothed() {
        Some(fps) => format!("FPS: {fps:.1}"),
        None => "FPS: warming".to_string(),
    };
    for mut text in &mut texts {
        **text = display.clone();
    }
}

pub fn update_error_list(
    log: Res<ErrorLog>,
    mut texts: Query<&mut Text, With<ErrorListText>>,
) {
    if !log.is_changed() && !log.is_added() {
        return;
    }
    for mut text in &mut texts {
        **text = log
            .0
            .iter()
            .map(|e| format!("[{:?}] {}", e.severity, e.message))
            .collect::<Vec<_>>()
            .join("\n");
    }
}

pub fn toggle_log_drawer(
    keys: Res<ButtonInput<KeyCode>>,
    mut drawers: Query<&mut Visibility, With<LogDrawer>>,
) {
    let kb_pressed =
        keys.just_pressed(KeyCode::Backquote) || keys.just_pressed(KeyCode::Backslash);
    // Mobile has no keyboard — the JS corner button calls
    // `rave_drawer_toggle()` which flips a thread-local flag the
    // wasm side of this module checks here.
    let js_pressed = take_js_toggle_request();
    if !kb_pressed && !js_pressed {
        return;
    }
    for mut vis in &mut drawers {
        *vis = match *vis {
            Visibility::Hidden => Visibility::Visible,
            _ => Visibility::Hidden,
        };
    }
}

#[cfg(target_arch = "wasm32")]
fn take_js_toggle_request() -> bool {
    js_toggle::PENDING.with(|c| c.replace(false))
}

#[cfg(not(target_arch = "wasm32"))]
fn take_js_toggle_request() -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
mod js_toggle {
    use std::cell::Cell;
    use wasm_bindgen::prelude::*;

    thread_local! {
        pub static PENDING: Cell<bool> = const { Cell::new(false) };
    }

    /// JS → Rust drawer toggle. The corner touch button in
    /// `web/index.html` calls this once per tap. The next Bevy Update
    /// tick's `toggle_log_drawer` consumes the flag and flips
    /// visibility. Without this affordance, mobile users can't see
    /// FPS / errors / net stats — keyboard is the only other path
    /// and mobile has no keyboard.
    #[wasm_bindgen]
    pub fn rave_drawer_toggle() {
        PENDING.with(|c| c.set(true));
    }
}

#[cfg(target_arch = "wasm32")]
pub fn update_net_stats(
    net: Res<bevy_libp2p::LayeNet>,
    remotes: Res<crate::remote_players::RemotePlayers>,
    mut texts: Query<&mut Text, With<NetStatsText>>,
) {
    let Some(mut t) = texts.iter_mut().next() else {
        return;
    };
    let id = &net.identity().0;
    let short = if id.len() > 10 { &id[id.len() - 10..] } else { id };
    t.0 = format!(
        "net: …{short} · peers={} · topic={}",
        remotes.0.len(),
        crate::POSITIONS_TOPIC,
    );
}

#[cfg(target_arch = "wasm32")]
pub fn update_clock(
    mut texts: Query<&mut Text, With<ClockText>>,
    mut log: ResMut<ErrorLog>,
) {
    let d = js_sys::Date::new_0();

    let Some(iso) = d.to_iso_string().as_string() else {
        log.emit(Severity::Error, "Date::to_iso_string() returned non-string");
        return;
    };
    let Some(time) = iso.get(11..23) else {
        log.emit(
            Severity::Error,
            format!("Date ISO too short ({} chars): {iso:?}", iso.len()),
        );
        return;
    };

    let Some(s) = d.to_string().as_string() else {
        log.emit(Severity::Error, "Date::to_string() returned non-string");
        return;
    };
    let Some(tz) = s.get(25..33) else {
        log.emit(
            Severity::Error,
            format!("Date string too short ({} chars): {s:?}", s.len()),
        );
        return;
    };

    for mut text in &mut texts {
        **text = format!("{time}  {tz}");
    }
}
