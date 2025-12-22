use std::fmt::Display;
use std::ops::{Add, Div, Mul, Sub};
use std::string::ToString;

use egui::{Color32, Event, FontSelection, Response, Sense, Ui, Vec2, Widget, text_edit, vec2};

// WheelBox -------------------------------------------------------------------
pub(crate) trait WheelBoxValue:
    ToString
    + Copy
    + Send
    + Sync
    + Add<Output = Self>
    + Sub<Output = Self>
    + Ord
    + Mul<Output = Self>
    + Div<Output = Self>
    + Display
    + 'static
{
    fn parse(text: &str) -> Result<Self, ()>;
    fn div2(self) -> Self;
    fn mul2(self) -> Self;
    fn mul_f32(self, f: f32) -> Self;
    fn one() -> Self;
}

#[derive(Clone)]
struct WheelBoxState<T> {
    state_value: T,
    string: String,
    single_step: T,
}

impl<T: WheelBoxValue> WheelBoxState<T> {
    fn new(value: T, single_step: T) -> Self {
        Self {
            state_value: value,
            string: value.to_string(),
            single_step,
        }
    }

    fn set_value(&mut self, value: T) {
        self.state_value = value;
        self.string = value.to_string();
    }
}

struct WheelsBoxKeys {
    q: bool,
    w: bool,
    r: bool,
    up: bool,
    down: bool,
}

pub struct WheelBox<'a, T> {
    default_single_step: Option<T>,
    always_update: bool,
    range: Option<[T; 2]>,
    suffix: Option<&'a str>,
    text_color: Option<Color32>,

    value: &'a mut T,
    single_step: Option<&'a mut T>,

    desired_width: Option<f32>,
}

impl<'a, T> WheelBox<'a, T> {
    pub fn new(value: &'a mut T) -> Self {
        Self {
            default_single_step: None,
            always_update: true,
            range: None,
            suffix: None,
            text_color: None,
            value,
            single_step: None,
            desired_width: None,
        }
    }

    pub fn range(mut self, range: Option<[T; 2]>) -> Self {
        self.range = range;
        self
    }

    pub fn text_color(mut self, color: Color32) -> Self {
        self.text_color = Some(color);
        self
    }

    pub fn suffix(mut self, suffix: &'static str) -> Self {
        self.suffix = Some(suffix);
        self
    }

    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    pub fn always_update(mut self, always_update: bool) -> Self {
        self.always_update = always_update;
        self
    }

    pub fn default_single_step(mut self, default_single_step: T) -> Self {
        self.default_single_step = Some(default_single_step);
        self
    }

    pub fn single_step(mut self, single_step: &'a mut T) -> Self {
        self.single_step = Some(single_step);
        self
    }
}

impl<'a, T> Widget for WheelBox<'a, T>
where
    T: WheelBoxValue,
{
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            default_single_step,
            always_update,
            range,
            suffix,
            text_color,
            value,
            single_step,
            desired_width,
        } = self;

        let id = ui.next_auto_id();
        let width = desired_width.unwrap_or(ui.available_width());

        let has_focus = ui.memory(|m| m.has_focus(id));

        let mut state = ui.data(|m| m.get_temp(id)).unwrap_or(WheelBoxState::new(
            *value,
            default_single_step.unwrap_or(T::one()),
        ));

        let visuals = ui.style().visuals.clone();
        let text_color = text_color.unwrap_or(visuals.widgets.active.text_color());

        // is focused -----------------------------------------------------
        let response = if has_focus {
            let keys = check_keys(ui);
            let tex_edit = text_edit::TextEdit::singleline(&mut state.string)
                .desired_width(width - 8.)
                .clip_text(true)
                .margin(egui::Margin::symmetric(4, 2));

            let mut response = tex_edit.ui(ui);

            if response.lost_focus() {
                // let v = state.string.parse::<i64>();
                let v = T::parse(&state.string);
                match v {
                    Ok(v) => {
                        if let Some(range) = range {
                            *value = v.clamp(range[0], range[1]);
                        } else {
                            *value = v;
                        }
                        state.set_value(*value);
                        response.mark_changed();
                    }
                    Err(_) => {
                        state.set_value(*value);
                    }
                }

                ui.data_mut(|m| {
                    m.insert_temp(response.id, state);
                });

                return response;
            }

            let single_step = if let Some(single_step) = single_step {
                if keys.q {
                    *single_step = single_step.div2();
                }
                if keys.w {
                    *single_step = single_step.mul2();
                }
                if keys.r {
                    *single_step = default_single_step.unwrap_or(T::one());
                }
                *single_step
            } else {
                if keys.q {
                    state.single_step = state.single_step.div2();
                }
                if keys.w {
                    state.single_step = state.single_step.mul2();
                }
                if keys.r {
                    state.single_step = default_single_step.unwrap_or(T::one());
                }
                state.single_step
            };

            if keys.up {
                let mut v = state.state_value + single_step;
                if let Some(ref range) = range {
                    v = v.min(range[1]);
                }

                state.set_value(v);
                if always_update {
                    *value = v;
                    response.mark_changed();
                }
            } else if keys.down {
                let mut v = state.state_value - single_step;
                if let Some(ref range) = range {
                    v = v.max(range[0]);
                }

                state.set_value(v);
                if always_update {
                    *value = v;
                    response.mark_changed();
                }
            }

            if response.hovered() {
                let (is_wheel, wheel) = check_wheel(ui);
                block_scroll_area(ui);

                if is_wheel {
                    let mut v = state.state_value + single_step.mul_f32(wheel);
                    if let Some(range) = range {
                        v = v.clamp(range[0], range[1]);
                    }

                    state.set_value(v);
                    if always_update {
                        *value = v;
                        response.mark_changed();
                    }
                }
            }

            ui.data_mut(|m| {
                m.insert_temp(response.id, state);
            });

            response

        // is not focused --------------------------------------------------
        } else {
            let font_id = FontSelection::default().resolve(ui.style());
            let row_height = ui.fonts_mut(|f| f.row_height(&font_id));
            let height = row_height + 6.;
            let mut save_state = false;

            let response = ui
                .allocate_response(vec2(width, height), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::Text);

            if response.clicked() || response.drag_started() {
                response.request_focus();
            }

            if response.lost_focus() {
                state.set_value(*value);

                save_state = true;
            }

            if response.hovered() {
                // let (is_wheel, wheel) = check_wheel(ui);

                // if is_wheel {
                //     block_sroll_area(ui);
                //     let mut v = *value + state.single_step.mul_f32(wheel);
                //     if let Some(range) = range {
                //         v = v.clamp(range[0], range[1]);
                //     }
                //     state.set_value(*value);
                //     response.request_focus();

                //     if always_update {
                //         *value = v;
                //         response.mark_changed();
                //     }

                //     save_state = true;
                // }

                ui.painter().rect_filled(
                    response.rect,
                    visuals.widgets.hovered.corner_radius,
                    visuals.widgets.hovered.bg_fill,
                );
            } else {
                ui.painter().rect_filled(
                    response.rect,
                    visuals.widgets.inactive.corner_radius,
                    visuals.widgets.inactive.bg_fill,
                );
            }

            if value != &state.state_value {
                state.set_value(*value);
                save_state = true;
            }

            let text = match suffix {
                Some(suffix) => format!("{} {}", state.string, suffix),
                None => state.string.clone(),
            };

            let galley = ui.painter().layout_no_wrap(text, font_id, text_color);

            const TEXT_PADDING: Vec2 = vec2(5., 3.);
            ui.painter().galley(
                response.rect.left_top() + TEXT_PADDING,
                galley,
                Color32::WHITE,
            );

            if save_state {
                ui.data_mut(|m| {
                    m.insert_temp(response.id, state);
                });
            }

            response
        };

        response
    }
}
// WhellBoxF ------------------------------------------------------------------
pub(crate) trait WheelBoxValueF:
    Copy
    + Send
    + Sync
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Display
    + PartialEq
    + 'static
{
    fn parse(text: &str) -> Result<Self, ()>;
    fn clip_value(self, range: [Self; 2]) -> Self;
    fn div2(self) -> Self;
    fn mul2(self) -> Self;
    fn mul_f32(self, f: f32) -> Self;
    fn min2(self, other: Self) -> Self;
    fn max2(self, other: Self) -> Self;
    fn one() -> Self;
}

#[derive(Clone)]
struct WheelBoxStateF<T> {
    state_value: T,
    string: String,
    single_step: T,
}

impl<T: Display + Copy> WheelBoxStateF<T> {
    fn new(value: T, single_step: T, decimals: u32) -> Self {
        Self {
            state_value: value,
            string: format!("{:.1$}", value, decimals as usize),
            single_step,
        }
    }

    fn set_value(&mut self, value: T, decimals: u32) {
        self.state_value = value;
        self.string = format!("{:.1$}", value, decimals as usize);
    }
}

pub struct WheelBoxF<'a, T> {
    default_single_step: Option<T>,
    decimals: u32,
    always_update: bool,
    range: Option<[T; 2]>,
    suffix: Option<&'a str>,
    text_color: Option<Color32>,

    value: &'a mut T,
    single_step: Option<&'a mut T>,

    desired_width: Option<f32>,
}

impl<'a, T> WheelBoxF<'a, T> {
    pub fn new(value: &'a mut T, decimals: u32) -> Self {
        Self {
            default_single_step: None,
            decimals,
            always_update: true,
            range: None,
            suffix: None,
            text_color: None,
            value,
            single_step: None,
            desired_width: None,
        }
    }

    pub fn range(mut self, range: Option<[T; 2]>) -> Self {
        self.range = range;
        self
    }

    pub fn text_color(mut self, color: Option<Color32>) -> Self {
        self.text_color = color;
        self
    }

    pub fn suffix(mut self, suffix: &'a str) -> Self {
        self.suffix = Some(suffix);
        self
    }

    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    pub fn always_update(mut self, always_update: bool) -> Self {
        self.always_update = always_update;
        self
    }

    pub fn default_single_step(mut self, default_single_step: T) -> Self {
        self.default_single_step = Some(default_single_step);
        self
    }

    pub fn single_step(mut self, single_step: &'a mut T) -> Self {
        self.single_step = Some(single_step);
        self
    }
}

impl<'a, T> Widget for WheelBoxF<'a, T>
where
    T: WheelBoxValueF,
{
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            default_single_step,
            decimals,
            always_update,
            range,
            suffix,
            text_color,
            value,
            single_step,
            desired_width,
        } = self;

        let id = ui.next_auto_id();
        let width = desired_width.unwrap_or(ui.available_width());

        let has_focus = ui.memory(|m| m.has_focus(id));

        let mut state = ui.data(|m| m.get_temp(id)).unwrap_or(WheelBoxStateF::new(
            *value,
            default_single_step.unwrap_or(T::one()),
            decimals,
        ));

        let visuals = ui.style().visuals.clone();
        let text_color = text_color.unwrap_or(visuals.widgets.active.text_color());

        // is focused -----------------------------------------------------
        let response = if has_focus {
            let keys = check_keys(ui);
            let tex_edit = text_edit::TextEdit::singleline(&mut state.string)
                .desired_width(width - 8.)
                .clip_text(true)
                .margin(egui::Margin::symmetric(4, 2));

            let mut response = tex_edit.ui(ui);

            if response.lost_focus() {
                state.string = state.string.replace(",", ".");
                // let v = state.string.parse::<f64>();
                let v = T::parse(&state.string);
                match v {
                    Ok(v) => {
                        if let Some(range) = range {
                            *value = v.clip_value(range);
                        } else {
                            *value = v;
                        }
                        state.set_value(*value, decimals);
                        response.mark_changed();
                    }
                    Err(_) => {
                        state.set_value(*value, decimals);
                    }
                }

                ui.data_mut(|m| {
                    m.insert_temp(response.id, state);
                });

                return response;
            }

            let single_step = if let Some(single_step) = single_step {
                if keys.q {
                    *single_step = single_step.div2();
                }
                if keys.w {
                    *single_step = single_step.mul2();
                }
                if keys.r {
                    *single_step = default_single_step.unwrap_or(T::one());
                }
                *single_step
            } else {
                if keys.q {
                    state.single_step = state.single_step.div2();
                }
                if keys.w {
                    state.single_step = state.single_step.mul2();
                }
                if keys.r {
                    state.single_step = default_single_step.unwrap_or(T::one());
                }
                state.single_step
            };

            if keys.up {
                let mut v = state.state_value + single_step;
                if let Some(ref range) = range {
                    v = v.min2(range[1]);
                }

                state.set_value(v, decimals);
                if always_update {
                    *value = v;
                    response.mark_changed();
                }
            } else if keys.down {
                let mut v = state.state_value - single_step;
                if let Some(ref range) = range {
                    v = v.max2(range[0]);
                }

                state.set_value(v, decimals);
                if always_update {
                    *value = v;
                    response.mark_changed();
                }
            }

            if response.hovered() {
                let (is_wheel, wheel) = check_wheel(ui);
                block_scroll_area(ui);

                if is_wheel {
                    let mut v = state.state_value + single_step.mul_f32(wheel);
                    if let Some(range) = range {
                        v = v.clip_value(range);
                    }

                    state.set_value(v, decimals);
                    if always_update {
                        *value = v;
                        response.mark_changed();
                    }
                }
            }

            ui.data_mut(|m| {
                m.insert_temp(response.id, state);
            });

            response

        // is not focused --------------------------------------------------
        } else {
            let font_id = FontSelection::default().resolve(ui.style());
            let row_height = ui.fonts_mut(|f| f.row_height(&font_id));
            let height = row_height + 6.;
            let mut save_state = false;

            let response = ui
                .allocate_response(vec2(width, height), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::Text);

            if response.clicked() || response.drag_started() {
                response.request_focus();
            }

            if response.lost_focus() {
                state.set_value(*value, decimals);

                save_state = true;
            }

            if response.hovered() {
                // let (is_wheel, wheel) = check_wheel(ui);

                // if is_wheel {
                //     block_sroll_area(ui);
                //     let mut v = *value + state.single_step.mul_f32(wheel);
                //     if let Some(range) = range {
                //         v = v.clip_value(range);
                //     }
                //     state.set_value(*value, decimals);
                //     response.request_focus();

                //     if always_update {
                //         *value = v;
                //         response.mark_changed();
                //     }

                //     save_state = true;
                // }

                ui.painter().rect_filled(
                    response.rect,
                    visuals.widgets.hovered.corner_radius,
                    visuals.widgets.hovered.bg_fill,
                );
            } else {
                ui.painter().rect_filled(
                    response.rect,
                    visuals.widgets.inactive.corner_radius,
                    visuals.widgets.inactive.bg_fill,
                );
            }

            if value != &state.state_value {
                state.set_value(*value, decimals);
                save_state = true;
            }

            let text = match suffix {
                Some(suffix) => format!("{} {}", state.string, suffix),
                None => state.string.clone(),
            };

            let galley = ui.painter().layout_no_wrap(text, font_id, text_color);

            const TEXT_PADDING: Vec2 = vec2(5., 3.);
            ui.painter().galley(
                response.rect.left_top() + TEXT_PADDING,
                galley,
                Color32::WHITE,
            );

            if save_state {
                ui.data_mut(|m| {
                    m.insert_temp(response.id, state);
                });
            }

            response
        };

        response
    }
}

// Helpers -------------------------------------------------------------------
fn check_wheel(ui: &Ui) -> (bool, f32) {
    ui.input(|inp| {
        for event in inp.events.iter() {
            if let Event::MouseWheel { delta, .. } = event {
                // if inp.viewport().focused.unwrap_or(false) {
                return (true, delta.y.clamp(-1., 1.));
                // }
                // return (false, 0.);
            }
        }
        (false, 0.)
    })
}

fn block_scroll_area(ui: &Ui) {
    ui.input_mut(|inp| {
        inp.smooth_scroll_delta = Vec2::ZERO;
    })
}

fn check_keys(ui: &mut Ui) -> WheelsBoxKeys {
    ui.input_mut(|inp| {
        let res = (
            inp.consume_key(egui::Modifiers::NONE, egui::Key::Q),
            inp.consume_key(egui::Modifiers::NONE, egui::Key::W),
            inp.consume_key(egui::Modifiers::NONE, egui::Key::R),
        );

        // TODO: make it more efficient
        if res.0 || res.1 || res.2 {
            inp.events = inp
                .events
                .clone()
                .into_iter()
                .filter(|event| {
                    if let egui::Event::Text(text) = event {
                        if text == "q" || text == "w" || text == "r" {
                            return false;
                        }
                    }
                    true
                })
                .collect();
        }

        WheelsBoxKeys {
            q: res.0,
            w: res.1,
            r: res.2,
            up: inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
            down: inp.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
        }
    })
}

// Implementations -----------------------------------------------------------
macro_rules! impl_wheel_box_value {
    ($($t:ty),*) => {
        $(
            impl WheelBoxValue for $t {
                #[inline]
                fn parse(text: &str) -> Result<Self, ()> {
                    text.parse().map_err(|_| ())
                }

                #[inline]
                fn div2(self) -> Self {
                    let v = self / 2;
                    v.max(1)
                }

                #[inline]
                fn mul2(self) -> Self {
                    self * 2
                }

                #[inline]
                fn mul_f32(self, f: f32) -> Self {
                    self * f as $t
                }

                #[inline]
                fn one() -> Self {
                    1
                }
            }
        )*
    };
}

impl_wheel_box_value!(i64, u64, i32, u32);

macro_rules! impl_wheel_box_value_f {
    ($($t:ty),*) => {
        $(
            impl WheelBoxValueF for $t {
                #[inline]
                fn parse(text: &str) -> Result<Self, ()> {
                    text.parse().map_err(|_| ())
                }

                #[inline]
                fn clip_value(self, range: [Self; 2]) -> Self {
                    self.clamp(range[0], range[1])
                }

                #[inline]
                fn div2(self) -> Self {
                    self / 2.
                }

                #[inline]
                fn mul2(self) -> Self {
                    self * 2.
                }

                #[inline]
                fn mul_f32(self, f: f32) -> Self {
                    self * f as $t
                }

                #[inline]
                fn min2(self, other: Self) -> Self {
                    self.min(other)
                }

                #[inline]
                fn max2(self, other: Self) -> Self {
                    self.max(other)
                }

                #[inline]
                fn one() -> Self {
                    1.
                }
            }
        )*
    };
}

impl_wheel_box_value_f!(f64, f32);
