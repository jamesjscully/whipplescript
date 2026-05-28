use std::env;

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        Some("--version") | Some("-V") => {
            println!("armature {}", armature_core::version());
        }
        Some("doctor") => {
            println!("armature doctor: {}", armature_kernel::kernel_stage());
        }
        Some("check") => {
            println!("armature check: {}", armature_parser::parser_stage());
        }
        _ => {
            println!("armature {}", armature_core::IMPLEMENTATION_STAGE);
        }
    }
}
