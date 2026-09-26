#[path = "../schema.rs"]
mod schema;
include!(concat!(env!("OUT_DIR"), "/module.rs"));

pub trait GeneratedProof {
    fn proof(&self) -> &'static str;
}
fn main() {
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    let reverse = std::env::args().nth(2).as_deref() == Some("reverse");
    if reverse {
        egui_states::build_scripts::generate_rust::<schema::Root<true>>(directory.join("rust"))
            .unwrap();
        egui_states::build_scripts::generate_python::<schema::Root<true>>(directory.join("python"))
            .unwrap();
    } else {
        egui_states::build_scripts::generate_rust::<schema::Root<false>>(directory.join("rust"))
            .unwrap();
        egui_states::build_scripts::generate_python::<schema::Root<false>>(
            directory.join("python"),
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_rust_and_custom_derive_are_usable() {
        let _: fn(
            &bindings::StatesServer,
            u16,
            Option<std::net::Ipv4Addr>,
            Option<String>,
        ) -> egui_states::server::Result<()> = bindings::StatesServer::start;
        let server = bindings::StatesServer::new().unwrap();
        assert_eq!(
            server.states.first.numbers.get().unwrap(),
            server.states.second.numbers.get().unwrap()
        );
        let payload = server.states.payload.get().unwrap();
        assert_eq!(payload.proof(), "derived implementation ran");
        assert_eq!(payload.title, "hello 🦀");
        assert_eq!(payload.fixed, [0, 17, u16::MAX]);
        assert!(payload.nested.as_ref().unwrap().enabled);
        assert_eq!(payload.choice, bindings::enums::Choice::High);
        assert_eq!(
            server.states.mapping.get().unwrap(),
            std::collections::HashMap::from([(1, 10), (3, 30), (20, 200)])
        );
        assert!(!server.is_running());
    }
}
