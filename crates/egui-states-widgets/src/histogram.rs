use egui::{Color32, Id, Mesh, Rect, Ui, pos2, vec2};

use egui_states::{ValueAtomic, ValueGraphs};

pub fn print_histogram(
    ui: &mut Ui,
    id: Id,
    histogram: &ValueGraphs<f32>,
    position: &ValueAtomic<(f32, f32)>,
) {
    let size = ui.available_size();
    const X_MARGIN: f32 = 12.;

    let (main_response, painter) = ui.allocate_painter(size, egui::Sense::click());
    let rect = main_response.rect;
    let w = rect.width() - (X_MARGIN * 2.0);
    let h = rect.height();

    let last_size: Option<(f32, f32, f32)> = ui.data(|d| d.get_temp(id));
    histogram.read(0, |hist, changed| {
        let mut redraw = changed;
        match last_size {
            None => redraw = true,
            Some((last_w, last_h, y_pos)) => {
                if last_w != w || last_h != h || rect.top() != y_pos {
                    redraw = true;
                }
            }
        }

        if !redraw {
            let last_mesh: Option<Option<Mesh>> = ui.data(|d| d.get_temp(id));
            if let Some(m) = last_mesh {
                if let Some(mesh) = m {
                    painter.add(mesh);
                }
                return;
            }
        }

        if let Some(hist) = hist {
            let hist_size = hist.y.len() as f32;

            let n_values = hist.y.len();
            let fill_color = Color32::from_gray(128);
            let y = rect.max.y;

            let mut mesh = egui::Mesh {
                indices: Vec::with_capacity((n_values - 1) * 6),
                vertices: Vec::with_capacity(n_values * 2),
                ..Default::default()
            };

            for (ind, v) in (&hist.y[0..n_values]).iter().enumerate() {
                // TODO: don't add 0 values
                let i = mesh.vertices.len() as u32;

                let pos = pos2(
                    ind as f32 / hist_size * w + rect.min.x + X_MARGIN,
                    rect.max.y - v * (h - 1.0) - 1.0,
                );

                mesh.colored_vertex(pos, fill_color);
                mesh.colored_vertex(pos2(pos.x, y), fill_color);

                mesh.add_triangle(i, i + 1, i + 2);
                mesh.add_triangle(i + 1, i + 2, i + 3);
            }

            let las_v = hist.y.last().unwrap();
            let i = n_values - 1;
            let pos = pos2(
                i as f32 / hist_size * w + rect.min.x + X_MARGIN,
                rect.max.y - las_v * (h - 1.0) - 1.0,
            );
            mesh.colored_vertex(pos, fill_color);
            mesh.colored_vertex(pos2(pos.x, y), fill_color);

            ui.data_mut(|d| {
                d.insert_temp(id, (w, h));
                d.insert_temp(id, Some(mesh.clone()));
            });
            painter.add(mesh);
        } else {
            ui.data_mut(|d| {
                d.insert_temp(id, (w, h));
                d.insert_temp(id, None::<Mesh>);
            });
        }
    });

    let hist_range = position.get();

    // min range
    painter.rect_filled(
        Rect::from_min_max(
            pos2(rect.min.x, rect.min.y),
            pos2(rect.min.x + X_MARGIN + hist_range.0 * w, rect.max.y),
        ),
        0.,
        Color32::from_rgba_premultiplied(0, 0, 32, 128),
    );

    let mut lines_rect = rect;
    lines_rect.min.x += X_MARGIN + hist_range.0 * w - 3.;
    lines_rect.max.x = lines_rect.min.x + 7.;
    lines_rect.max.y -= 10.;

    let grap_rect =
        Rect::from_center_size(pos2(lines_rect.center().x, rect.max.y - 5.), vec2(10., 10.));
    let response_grap = ui
        .allocate_rect(grap_rect, egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

    painter.rect_filled(lines_rect.shrink2(vec2(3., 0.)), 0., Color32::WHITE);

    lines_rect.min.y += 10.;
    let response_line = ui
        .allocate_rect(lines_rect, egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

    let mut save = false;
    let mut new_range = hist_range.clone();

    if let Some(pos) = response_line.interact_pointer_pos() {
        if hist_range.0 == 0.0 && pos.x < main_response.rect.min.x + X_MARGIN {
            // do nothing
        } else if hist_range.0 == hist_range.1 && pos.x > response_line.rect.max.x {
            // do nothing
        } else {
            let delta = response_line.drag_delta().x / w;
            new_range.0 = (hist_range.0 + delta).clamp(0.0, hist_range.1);
            save = true;
        }
    }

    if let Some(pos) = response_grap.interact_pointer_pos() {
        if hist_range.0 == 0.0 && pos.x < main_response.rect.min.x + X_MARGIN {
            // do nothing
        } else if hist_range.0 == hist_range.1 && pos.x > response_line.rect.max.x {
            // do nothing
        } else {
            let delta = response_grap.drag_delta().x / w;
            new_range.0 = (hist_range.0 + delta).clamp(0.0, hist_range.1);
            save = true;
        }
    }

    // max range
    painter.rect_filled(
        Rect::from_min_max(
            pos2(rect.min.x + X_MARGIN + hist_range.1 * w, rect.min.y),
            pos2(rect.max.x, rect.max.y),
        ),
        0.,
        Color32::from_rgba_premultiplied(0, 0, 32, 128),
    );

    // paint here to cover max range zone
    painter.circle_filled(grap_rect.center(), 5., Color32::WHITE);

    let mut lines_rect = rect;
    lines_rect.min.x += X_MARGIN + hist_range.1 * w - 3.;
    lines_rect.max.x = lines_rect.min.x + 7.;
    lines_rect.min.y += 10.;

    let grap_rect = Rect::from_center_size(
        pos2(lines_rect.center().x, rect.min.y + 5.),
        vec2(10., 10.),
    );
    let response_grap = ui
        .allocate_rect(grap_rect, egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

    painter.rect_filled(lines_rect.shrink2(vec2(3., 0.)), 0., Color32::WHITE);

    lines_rect.max.y -= 10.;
    let response_line = ui
        .allocate_rect(lines_rect, egui::Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

    painter.circle_filled(
        pos2(response_line.rect.center().x, rect.min.y + 5.),
        5.,
        Color32::WHITE,
    );

    if let Some(pos) = response_line.interact_pointer_pos() {
        if hist_range.1 == 1.0 && pos.x > main_response.rect.max.x - X_MARGIN {
            // do nothing
        } else if hist_range.1 == hist_range.0 && pos.x < response_line.rect.min.x {
            // do nothing
        } else {
            let delta = response_line.drag_delta().x / w;
            new_range.1 = (hist_range.1 + delta).clamp(hist_range.0, 1.0);
            save = true;
        }
    }

    if let Some(pos) = response_grap.interact_pointer_pos() {
        if hist_range.1 == 1.0 && pos.x > main_response.rect.max.x - X_MARGIN {
            // do nothing
        } else if hist_range.1 == hist_range.0 && pos.x < response_line.rect.min.x {
            // do nothing
        } else {
            let delta = response_grap.drag_delta().x / w;
            new_range.1 = (hist_range.1 + delta).clamp(hist_range.0, 1.0);
            save = true;
        }
    }

    if save {
        position.set_signal(new_range);
    }
}
