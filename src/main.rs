use tsot::proto::Card;

fn main() {
    println!("tsot engine. Card type ready: {:?}", std::any::type_name::<Card>());
}
