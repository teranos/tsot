use bevy::asset::AssetMetaCheck;
use bevy::prelude::*;
use bevy::window::WindowPlugin;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub fn run() {
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::srgb(0.0, 1.0, 0.0)))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "universe".to_string(),
                        canvas: Some("#bevy".to_owned()),
                        fit_canvas_to_parent: true,
                        prevent_default_event_handling: false,
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                }),
        )
        .add_systems(Startup, setup);

    #[cfg(target_arch = "wasm32")]
    app.add_systems(Update, update_clock);

    app.run();
}

#[derive(Component)]
struct ClockText;

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);

    commands.spawn((
        Text::new("HH:MM:SS.mmm  GMT±HHMM"),
        ClockText,
        TextFont {
            font_size: 11.0,
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
        Text::new("universe · dev"),
        TextFont {
            font_size: 11.0,
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

#[cfg(target_arch = "wasm32")]
fn update_clock(mut texts: Query<&mut Text, With<ClockText>>) {
    let d = js_sys::Date::new_0();
    let iso = d.to_iso_string().as_string().unwrap_or_default();
    let time = iso.get(11..23).unwrap_or("");
    let s = d.to_string().as_string().unwrap_or_default();
    let tz = s.get(25..33).unwrap_or("");
    for mut text in &mut texts {
        **text = format!("{time}  {tz}");
    }
}
