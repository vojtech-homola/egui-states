//! One registration table for Cargo test names and Python scenario selection.

#[macro_export]
macro_rules! for_each_scenario {
    ($callback:ident) => {
        $callback! {
            sync => "sync",
            takes => "takes",
            blocking_value => "blocking-value",
            blocking_empty => "blocking-empty",
            blocking_data => "blocking-data",
            blocking_multi => "blocking-multi",
            cache => "cache",
            wire => "wire",
            take_controls => "take-controls",
            cache_replacement => "cache-replacement",
            disconnect_pending => "disconnect-pending",
            blocking_remove => "blocking-remove",
            blocking_reset => "blocking-reset",
        }
    };
}

macro_rules! scenario_names {
    ($($name:ident => $scenario:literal),* $(,)?) => {
        pub const SCENARIOS: &[&str] = &[$($scenario),*];
    };
}

for_each_scenario!(scenario_names);
