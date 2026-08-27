use std::mem::size_of;

#[derive(Debug)]
enum Node {
    Number(u64),
    Binary { left: Box<Node>, right: Box<Node>, opcode: u8 },
    Text([u8; 96]),
}

fn main() {
    let node = Node::Binary {
        left: Box::new(Node::Number(1)),
        right: Box::new(Node::Text([0; 96])),
        opcode: 3,
    };
    if let Node::Binary { opcode, .. } = node { assert_eq!(opcode, 3); }
    println!("instances=2000000 node_size={}", size_of::<Node>());
}
