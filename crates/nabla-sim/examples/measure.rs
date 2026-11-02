//! The platform-dependent measurements of `docs/multiplayer.typ`, natively:
//! `cargo run --release -p nabla-sim --example measure`. The browser runs the same code.

#![expect(clippy::print_stdout, reason = "the report is this program's output")]

fn main() {
    println!("{}", nabla_sim::measure::report(1, 7200, 3000));
}
