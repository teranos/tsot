//! Bevy plugin atop `laye-net`.

use bevy_app::{App, Plugin, Update};
use bevy_ecs::prelude::*;
use laye_me::load_or_fresh;
use laye_net::{Net, NetConfig};
use std::ops::Deref;

pub use laye_me::Keypair;
pub use laye_net::{NetError, NetEvent};
pub use laye_protocol::{PeerId, Topic};

pub struct LibP2PPlugin {
    pub bootstrap_addrs: Vec<String>,
    pub identity_bytes: Option<Vec<u8>>,
    pub topics: Vec<Topic>,
    pub identify_protocol: String,
}

#[derive(Resource)]
pub struct LayeNet(Net);

impl Deref for LayeNet {
    type Target = Net;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Message, Debug, Clone)]
pub struct LibP2PMessage(pub NetEvent);

impl Plugin for LibP2PPlugin {
    fn build(&self, app: &mut App) {
        let keypair = load_or_fresh(self.identity_bytes.as_deref())
            .expect("laye-me identity load");

        let (net, drive) = laye_net::new(NetConfig {
            bootstrap_addrs: self.bootstrap_addrs.clone(),
            keypair,
            topics: self.topics.clone(),
            identify_protocol: self.identify_protocol.clone(),
        })
        .expect("laye-net new");

        spawn_drive(drive);

        app.insert_resource(LayeNet(net));
        app.add_message::<LibP2PMessage>();
        app.add_systems(Update, drain_events);
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn_drive(drive: laye_net::NetDrive) {
    wasm_bindgen_futures::spawn_local(drive);
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_drive(drive: laye_net::NetDrive) {
    tokio::spawn(drive);
}

fn drain_events(net: Res<LayeNet>, mut writer: MessageWriter<LibP2PMessage>) {
    for ev in net.poll_events() {
        writer.write(LibP2PMessage(ev));
    }
}
