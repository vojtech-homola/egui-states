use egui::{Color32, Image, ImageSource, Rect, Response, Sense, Ui, Vec2, Widget, vec2};

pub enum ButtonContent {
    Icon(&'static ImageSource<'static>),
    Text(String),
    TextIcon(String, &'static ImageSource<'static>),
}

impl From<&'static str> for ButtonContent {
    #[inline]
    fn from(text: &'static str) -> Self {
        ButtonContent::Text(text.to_string())
    }
}

impl From<&'static ImageSource<'static>> for ButtonContent {
    #[inline]
    fn from(icon: &'static ImageSource<'static>) -> Self {
        ButtonContent::Icon(icon)
    }
}

impl From<(&'static str, &'static ImageSource<'static>)> for ButtonContent {
    #[inline]
    fn from((text, icon): (&'static str, &'static ImageSource<'static>)) -> Self {
        ButtonContent::TextIcon(text.to_string(), icon)
    }
}

impl From<String> for ButtonContent {
    #[inline]
    fn from(text: String) -> Self {
        ButtonContent::Text(text)
    }
}

pub struct SelectableButton {
    selected: bool,
    content: ButtonContent,
    size: Vec2,
    text_color: Option<Color32>,
    background_color: Option<Color32>,
}

impl SelectableButton {
    pub fn new(selected: bool, content: impl Into<ButtonContent>, size: Vec2) -> Self {
        Self {
            selected,
            content: content.into(),
            size,
            text_color: None,
            background_color: None,
        }
    }

    pub fn text_color(mut self, color: Option<Color32>) -> Self {
        self.text_color = color;
        self
    }

    pub fn background_color(mut self, color: Option<Color32>) -> Self {
        self.background_color = color;
        self
    }
}

impl Widget for SelectableButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            selected,
            content,
            size,
            text_color: color,
            background_color,
        } = self;

        let (rect, response) = ui.allocate_at_least(size, Sense::click());

        if ui.is_rect_visible(response.rect) {
            if selected || response.hovered() || response.highlighted() || response.has_focus() {
                let mut visuals = ui.style().interact_selectable(&response, selected);
                visuals.bg_stroke.width = 1.0;
                let rect = rect.expand(visuals.expansion);

                let bg_color = match background_color {
                    Some(color) => color,
                    None => visuals.bg_fill,
                };

                ui.painter().rect(
                    rect,
                    visuals.corner_radius,
                    bg_color,
                    visuals.bg_stroke,
                    egui::StrokeKind::Inside,
                );
            }

            match content {
                ButtonContent::Icon(icon) => {
                    let icon_size = rect.width().min(rect.height());
                    let position = rect.center_top() - vec2(icon_size / 2.0, 0.0);

                    let mut tex_options = egui::TextureOptions::default();
                    tex_options.magnification = egui::TextureFilter::Nearest;

                    let image = Image::new(icon.clone()).texture_options(tex_options);
                    image.paint_at(
                        ui,
                        Rect::from_min_size(position, vec2(icon_size, icon_size)),
                    );
                }
                ButtonContent::Text(text) => {
                    let font_id = egui::FontSelection::default().resolve(ui.style());
                    let color = match color {
                        Some(color) => color,
                        None => ui.visuals().text_color(),
                    };
                    let galley = ui
                        .painter()
                        .layout_no_wrap(text.to_string(), font_id, color);

                    let mut pos = rect.left_top();
                    pos.x += (rect.width() - galley.size().x) / 2.0;
                    pos.y += (rect.height() - galley.size().y) / 2.0;
                    ui.painter().galley(pos, galley, color);
                }
                ButtonContent::TextIcon(text, icon) => {
                    let font_id = egui::FontSelection::default().resolve(ui.style());
                    let color = match color {
                        Some(color) => color,
                        None => ui.visuals().text_color(),
                    };
                    let galley = ui
                        .painter()
                        .layout_no_wrap(text.to_string(), font_id, color);

                    let start_pos = rect.left_top();
                    let icon_size = rect.height();
                    let text_size = galley.size().x;
                    let all_size = icon_size + text_size + 1.0;

                    let mut text_pos = start_pos;
                    text_pos.x += (rect.width() - all_size) / 2.0;
                    text_pos.y += (rect.height() - galley.size().y) / 2.0;

                    ui.painter().galley(text_pos, galley, color);

                    let mut icon_pos = start_pos;
                    icon_pos.x = text_pos.x + text_size + 1.0;

                    let mut tex_options = egui::TextureOptions::default();
                    tex_options.magnification = egui::TextureFilter::Nearest;

                    let image = Image::new(icon.clone()).texture_options(tex_options);
                    image.paint_at(
                        ui,
                        Rect::from_min_size(icon_pos, vec2(icon_size, icon_size)),
                    );
                }
            }
        }

        response
    }
}

#[inline]
pub fn selec_butt<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    content: impl Into<ButtonContent>,
    size: Vec2,
) -> bool {
    let response = ui.add(SelectableButton::new(*current == value, content, size));
    if response.clicked() {
        *current = value;
        return true;
    }
    false
}

#[inline]
pub fn selec_butt_enabled<T: PartialEq>(
    ui: &mut Ui,
    current: &mut T,
    value: T,
    content: impl Into<ButtonContent>,
    size: Vec2,
    enabled: bool,
) -> Response {
    let response = ui.add_enabled(
        enabled,
        SelectableButton::new(*current == value, content, size),
    );
    if response.clicked() {
        *current = value;
    }
    response
}

// #[inline]
// pub fn single_select<T: PartialEq>(
//     ui: &mut Ui,
//     current: &mut T,
//     value: T,
//     none_value: T,
//     content: impl Into<ButtonContent>,
//     size: Vec2,
// ) -> Response {
//     let response = ui.add(SelectableButton::new(*current == value, content, size));
//     if response.clicked() {
//         if *current == value {
//             *current = none_value;
//         } else {
//             *current = value;
//         }
//     }

//     response
// }

pub struct SingleSelectButt<'a, T: PartialEq> {
    ui: &'a mut Ui,
    current: &'a mut T,
    none_value: T,
}

impl<'a, T: PartialEq + Copy> SingleSelectButt<'a, T> {
    pub fn new(ui: &'a mut Ui, current: &'a mut T, none_value: T) -> Self {
        Self {
            ui,
            current,
            none_value,
        }
    }

    pub fn add(&mut self, value: T, content: impl Into<ButtonContent>, size: Vec2) -> Response {
        let response = self
            .ui
            .add(SelectableButton::new(*self.current == value, content, size));
        if response.clicked() {
            if *self.current == value {
                *self.current = self.none_value;
            } else {
                *self.current = value;
            }
        }

        response
    }
}

#[inline]
pub fn multi_select(
    ui: &mut Ui,
    values: bool,
    content: impl Into<ButtonContent>,
    size: Vec2,
    clicked: impl FnOnce(bool),
) -> Response {
    let response = ui.add(SelectableButton::new(values, content, size));
    if response.clicked() {
        clicked(!values);
    }

    response
}
