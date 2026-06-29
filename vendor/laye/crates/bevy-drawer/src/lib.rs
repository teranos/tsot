use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use bevy_observability::{ErrorLog, Severity};

#[derive(Resource, Default)]
pub struct DrawerView {
    pub lines: Vec<String>,
    pub open: bool,
}

pub struct DrawerPlugin;

impl Plugin for DrawerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DrawerView::default());
        app.add_systems(Update, refresh_view);
    }
}

fn refresh_view(log: Res<ErrorLog>, mut view: ResMut<DrawerView>) {
    view.lines = log
        .0
        .iter()
        .map(|e| format!("[{}] {}", severity_tag(e.severity), e.message))
        .collect();
}

fn severity_tag(s: Severity) -> &'static str {
    match s {
        Severity::Note => "note",
        Severity::Warn => "warn",
        Severity::Error => "error",
    }
}
