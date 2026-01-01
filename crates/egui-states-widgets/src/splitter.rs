use egui::{vec2, CursorIcon, Id, Sense, Ui, UiBuilder};

// orientation of the splitter
#[derive(Debug, Clone, Copy)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

pub struct Splitter {
    id: Id,
    orientation: Orientation,
    initial_size: f32,
    min_size: f32,
}

impl Splitter {
    pub fn new(id: Id, orientation: Orientation) -> Self {
        Self {
            id,
            orientation,
            initial_size: 0.5,
            min_size: 0.05,
        }
    }

    pub fn set_sizes(mut self, initial_size: f32, min_size: f32) -> Self {
        self.initial_size = initial_size;
        self.min_size = min_size;
        self
    }

    pub fn horizontal(id: Id) -> Self {
        Self::new(id, Orientation::Horizontal)
    }

    pub fn vertical(id: Id) -> Self {
        Self::new(id, Orientation::Vertical)
    }

    // TODO: input ui has to be empty in thi implementation
    pub fn show(
        self,
        ui: &mut Ui,
        first_ui: impl FnOnce(&mut Ui),
        second_ui: impl FnOnce(&mut Ui),
    ) {
        let mut save = false;
        let mut size = ui.data(|d| d.get_temp(self.id)).unwrap_or_else(|| {
            save = true;
            self.initial_size
        });

        let mut a_rect = ui.max_rect();
        let mut b_rect = ui.max_rect();

        const SEPARATOR_SIZE: f32 = 1.;

        match self.orientation {
            Orientation::Horizontal => {
                let size = a_rect.width() * size;
                a_rect.max.x = a_rect.min.x + size;
                b_rect.min.x = a_rect.max.x + SEPARATOR_SIZE;
            }
            Orientation::Vertical => {
                let size = a_rect.height() * size;
                a_rect.max.y = a_rect.min.y + size;
                b_rect.min.y = a_rect.max.y + SEPARATOR_SIZE;
            }
        }

        first_ui(&mut ui.new_child(UiBuilder::new().max_rect(a_rect)));
        second_ui(&mut ui.new_child(UiBuilder::new().max_rect(b_rect)));

        let mut sep_rect = ui.max_rect();
        let drag_rect = match self.orientation {
            Orientation::Horizontal => {
                sep_rect.min.x = a_rect.max.x;
                sep_rect.max.x = b_rect.min.x;
                sep_rect.expand2(vec2(4. , 0.))
            }
            Orientation::Vertical => {
                sep_rect.min.y = a_rect.max.y;
                sep_rect.max.y = b_rect.min.y;
                sep_rect.expand2(vec2(0., 4.))
            }
        };

        let sep_response = ui.allocate_rect(drag_rect, Sense::drag());

        if sep_response.hovered() {
            match self.orientation {
                Orientation::Horizontal => ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal),
                Orientation::Vertical => ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical),
            }
        }

        if sep_response.dragged() {
            let (delta_pos, extended_rect) = match self.orientation {
                Orientation::Horizontal => (
                    sep_response.drag_delta().x / ui.max_rect().width(),
                    sep_rect.expand2(vec2(2., 0.)),
                ),
                Orientation::Vertical => (
                    sep_response.drag_delta().y / ui.max_rect().height(),
                    sep_rect.expand2(vec2(0., 2.)),
                ),
            };

            size += delta_pos;
            size = size.clamp(self.min_size, 1. - self.min_size);
            save = true;

            ui.painter()
                .rect_filled(extended_rect, 0., egui::Color32::BLUE);
        } else {
            ui.painter()
                .rect_filled(sep_rect, 0., egui::Color32::DARK_GRAY);
        }

        if save {
            ui.data_mut(|d| {
                d.insert_temp(self.id, size);
            });
        }
    }
}
