#[path = "../awkward.rs"]
mod awkward;
#[path = "../schema.rs"]
mod schema;
include!(concat!(env!("OUT_DIR"), "/module.rs"));

pub trait GeneratedProof {
    fn proof(&self) -> &'static str;
}
fn main() {
    let directory = std::path::PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    let mode = std::env::args().nth(2);
    if mode.as_deref() == Some("awkward") {
        // One Python package per kind, so each failure is reported separately.
        use egui_states::build_scripts::generate_python;
        generate_python::<awkward::KeywordNames>(directory.join("keywords")).unwrap();
        generate_python::<awkward::SpecialLiterals>(directory.join("literals")).unwrap();
        generate_python::<awkward::UnitStruct>(directory.join("unit")).unwrap();
    } else {
        egui_states::build_scripts::generate_rust::<schema::Root>(directory.join("rust")).unwrap();
        egui_states::build_scripts::generate_python::<schema::Root>(directory.join("python"))
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
        let header = bindings::structs::Inner { enabled: true };
        let measurements = &server.states.measurements;
        measurements
            .set_with_header(&[1.0, 2.0], &header, false)
            .unwrap();
        assert_eq!(measurements.header().unwrap(), Some(header));
        assert!(!server.is_running());

        let initials = &server.states.initials;
        assert_eq!(initials.gain.get().unwrap(), 0.5);
        assert_eq!(initials.status.get().unwrap(), "ready");
        assert_eq!(
            initials.choice.get().unwrap(),
            bindings::enums::Choice::High
        );
        assert_eq!(
            initials.inner.get().unwrap(),
            bindings::structs::Inner { enabled: true }
        );
        assert_eq!(initials.counter.get().unwrap(), 7);
        assert!(initials.flag.get().unwrap());
        assert_eq!(initials.plain.get().unwrap(), 0);
    }

    #[cfg(feature = "awkward")]
    #[test]
    fn awkward_names_and_literals_survive_rust_generation() {
        let server = awkward_bindings::StatesServer::new().unwrap();
        let (keywords, literals) = (&server.states.keywords, &server.states.literals);
        assert_eq!(
            keywords.keyword.get().unwrap(),
            awkward_bindings::enums::Keyword::True
        );
        let reserved = keywords.reserved.get().unwrap();
        assert_eq!((reserved.from, reserved.lambda), (3, true));
        assert_eq!(keywords.global.get().unwrap(), 1);
        let raw = keywords.r#type.get().unwrap();
        assert_eq!((raw.r#type, raw.r#in), (5, true));
        assert_eq!(
            keywords.raw_command.get().unwrap(),
            awkward_bindings::enums::RawCommand::r#match {
                r#type: 1,
                r#if: true
            }
        );
        // The generated `Option` and `String` enums leave the standard types
        // of their neighbours alone.
        use awkward_bindings::enums as generated;
        assert!(keywords.option.get().unwrap() > generated::Option::Off);
        assert!(matches!(
            keywords.short.get().unwrap(),
            generated::s::Second
        ));
        match keywords.shadowed.get().unwrap() {
            generated::String::r#final {
                text,
                maybe: Some(generated::s::Second),
                choices,
            } => {
                assert_eq!(text, "text");
                assert!(matches!(choices[..], [generated::Option::On]));
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            keywords.shadowed_map.get().unwrap(),
            generated::String::IntEnum([(1, "one".to_string())].into())
        );
        assert_eq!(literals.escaped.get().unwrap(), awkward::ESCAPED);
        assert!(literals.not_a_number.get().unwrap().is_nan());
        assert_eq!(literals.positive_infinity.get().unwrap(), f64::INFINITY);
        assert_eq!(literals.negative_infinity.get().unwrap(), f64::NEG_INFINITY);
        assert!(literals.single_not_a_number.get().unwrap().is_nan());
        server.states.unit.marker.get().unwrap();

        // Names that are keywords only in Python are plain names here.
        let types = &server.states.keyword_types;
        assert_eq!(types.nothing.get().unwrap(), generated::None::Something);
        assert_eq!(
            types.truth.get().unwrap(),
            generated::True::Maybe(Some(generated::None::Something))
        );
        let function = types.function.get().unwrap();
        assert_eq!(
            (function.value, function.choice, function.maybe),
            (2, generated::None::Something, Option::None)
        );
    }
}
