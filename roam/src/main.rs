fn main() {
    let mut w = roam::world::World::new();
    w.step(roam::world::INPUT_D, 100.0);
    w.step(roam::world::INPUT_S, 100.0);
    println!("roam: {}", w.state_json());
}
