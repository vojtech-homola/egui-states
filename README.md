# egui-states: a server/client for synchronizing states between server and egui UIs

## Custom types

Use `#[egui_states::typed]` to make a struct or enum available to
`egui-states`. The attribute implements `Typed` and derives Serde serialization
through the library's Serde re-export, so the consuming crate does not need a
direct `serde` dependency.

```rust
#[egui_states::typed]
#[derive(Clone, Debug, PartialEq)]
struct Settings {
    enabled: bool,
    label: String,
}
```

This replaces the former
`#[derive(serde::Serialize, serde::Deserialize, egui_states::Typed)]` syntax.
