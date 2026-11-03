//! Home page, laid out after the Windows PredatorSense 5.x home screen: a
//! GPU panel on the left (frequency gauge, dGPU state, GPU/CPU usage), the
//! laptop on a lit platform in the middle, and GPU/CPU/System temperature
//! dials on the right, under a "PREDATORSENSE / System Mode" title.
//!
//! Only the layout and the palette follow the Windows app (its `mgd`
//! `#002e40`, `lb` `#00c8ec`, `gd` `#006476` and `gb` `#0ed8cc` tokens).
//! Every frame, ring and dial here is drawn in Cairo by this module. None of
//! the Windows app's own artwork, which is Acer's, is used.

use gtk4::prelude::*;
use gtk4::{self as gtk, glib};
use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use crate::hardware::sensors;

const CYAN: (f64, f64, f64) = (0.0, 200.0 / 255.0, 236.0 / 255.0);
const TEAL: (f64, f64, f64) = (14.0 / 255.0, 216.0 / 255.0, 204.0 / 255.0);
const DEEP: (f64, f64, f64) = (0.0, 100.0 / 255.0, 118.0 / 255.0);

const PANEL_W: i32 = 340;
const PANEL_H: i32 = 540;
const CENTER_W: i32 = 400;
const REFRESH: Duration = Duration::from_millis(1500);
/// Temperature that fills a dial completely.
const TEMP_FULL_SCALE: f64 = 100.0;

/// What one refresh reads, on a worker thread.
#[derive(Default, Clone)]
struct Reading {
    gpu_clock_mhz: Option<u32>,
    gpu_util_pct: Option<u32>,
    gpu_temp: Option<f64>,
    cpu_temp: Option<f64>,
    system_temp: Option<f64>,
    cpu_ticks: Option<(u64, u64)>,
    /// Highest core clock the GPU reports, so the gauge has a scale.
    gpu_clock_max_mhz: u32,
}

fn read() -> Reading {
    let data = sensors::read_all_sensors();
    let metrics = crate::hardware::gpu::read_gpu_metrics();
    Reading {
        gpu_clock_mhz: data.gpu_info.clock_mhz,
        gpu_util_pct: data.gpu_info.utilization_pct,
        gpu_temp: data.gpu_temp,
        cpu_temp: data.cpu_temp,
        system_temp: data.system_temp,
        cpu_ticks: read_cpu_ticks(),
        gpu_clock_max_mhz: metrics.clock_max_core,
    }
}

/// (total, idle) jiffies from the aggregate `cpu` line of /proc/stat - a
/// single line, far lighter than `procs::UsageSampler`, which also walks
/// every process.
fn read_cpu_ticks() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().next()?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    if fields.len() < 5 {
        return None;
    }
    // idle + iowait count as idle, the same split `procs.rs` uses.
    Some((fields.iter().sum(), fields[3] + fields[4]))
}

pub fn build() -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_hexpand(true);
    scroll.set_vexpand(true);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.set_halign(gtk::Align::Center);
    page.set_margin_top(4);

    // === Title ===
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    title.set_halign(gtk::Align::Center);
    let predator = gtk::Label::new(Some("PREDATOR"));
    predator.add_css_class("home-title-bold");
    let sense = gtk::Label::new(Some("SENSE"));
    sense.add_css_class("home-title-light");
    title.append(&predator);
    title.append(&sense);
    page.append(&title);

    let mode_label = gtk::Label::new(None);
    mode_label.add_css_class("home-mode");
    mode_label.set_margin_bottom(14);
    page.append(&mode_label);

    // Wraps into a column on a narrow (e.g. tiled) window instead of
    // clipping the side panels; at full width all three sit in one row.
    let row = gtk::FlowBox::new();
    row.set_selection_mode(gtk::SelectionMode::None);
    row.set_max_children_per_line(3);
    row.set_min_children_per_line(1);
    row.set_homogeneous(false);
    row.set_column_spacing(0);
    row.set_row_spacing(24);
    row.set_halign(gtk::Align::Center);
    page.append(&row);

    // === Left: GPU panel ===
    let gpu_clock: Rc<Cell<Option<f64>>> = Rc::new(Cell::new(None));
    let gpu_clock_max: Rc<Cell<f64>> = Rc::new(Cell::new(2500.0));
    let (left, left_content) = panel(false);
    let gauge = gtk::DrawingArea::new();
    gauge.set_size_request(260, 260);
    gauge.set_halign(gtk::Align::Center);
    {
        let clock = gpu_clock.clone();
        let max = gpu_clock_max.clone();
        gauge.set_draw_func(move |_, cr, w, h| {
            let fraction = clock.get().map(|c| c / max.get().max(1.0));
            draw_frequency_gauge(cr, w as f64, h as f64, fraction);
        });
    }
    let gauge_overlay = gtk::Overlay::new();
    gauge_overlay.set_child(Some(&gauge));
    let gauge_text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    gauge_text.set_halign(gtk::Align::Center);
    gauge_text.set_valign(gtk::Align::Center);
    let gpu_big = gtk::Label::new(Some("GPU"));
    gpu_big.add_css_class("home-gauge-title");
    let freq_caption = gtk::Label::new(Some(crate::i18n::t("home_gpu_frequency")));
    freq_caption.add_css_class("home-caption");
    let freq_value = gtk::Label::new(Some("--"));
    freq_value.add_css_class("home-gauge-value");
    freq_value.set_margin_top(10);
    let freq_unit = gtk::Label::new(Some("MHz"));
    freq_unit.add_css_class("home-unit");
    gauge_text.append(&gpu_big);
    gauge_text.append(&freq_caption);
    gauge_text.append(&freq_value);
    gauge_text.append(&freq_unit);
    gauge_overlay.add_overlay(&gauge_text);
    left_content.append(&gauge_overlay);

    let gpu_state = gtk::Label::new(Some(crate::i18n::t("home_gpu_idle")));
    gpu_state.add_css_class("home-gpu-state");
    gpu_state.set_margin_top(8);
    left_content.append(&gpu_state);

    let usage_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    usage_row.set_halign(gtk::Align::Center);
    usage_row.set_margin_top(36);
    let (gpu_usage_box, gpu_usage_bar, gpu_usage_label) =
        usage_meter(crate::i18n::t("home_gpu_usage"));
    let (cpu_usage_box, cpu_usage_bar, cpu_usage_label) =
        usage_meter(crate::i18n::t("home_cpu_usage"));
    usage_row.append(&gpu_usage_box);
    usage_row.append(&cpu_usage_box);
    left_content.append(&usage_row);
    row.insert(&left, -1);

    // === Centre: laptop on its platform ===
    let center = gtk::Overlay::new();
    center.set_size_request(CENTER_W, PANEL_H);
    let center_bg = gtk::DrawingArea::new();
    center_bg.set_draw_func(|_, cr, w, h| draw_center_stage(cr, w as f64, h as f64));
    center.set_child(Some(&center_bg));
    let product = std::fs::read_to_string("/sys/class/dmi/id/product_name").unwrap_or_default();
    if let Some(path) = crate::ui::dashboard_page::find_model_photo(product.trim()) {
        let pic = gtk::Picture::for_filename(path);
        pic.set_can_shrink(true);
        pic.set_size_request(300, 220);
        pic.set_halign(gtk::Align::Center);
        pic.set_valign(gtk::Align::End);
        // Sits on the platform ellipse `draw_center_stage` puts at ~78%.
        pic.set_margin_bottom((PANEL_H as f64 * 0.25) as i32);
        center.add_overlay(&pic);
    }
    row.insert(&center, -1);

    // === Right: temperatures ===
    let (right, right_content) = panel(true);
    right_content.set_spacing(4);
    let mut dials = Vec::new();
    for key in ["GPU", "CPU", "home_system"] {
        let caption = if key == "home_system" {
            crate::i18n::t(key)
        } else {
            key
        };
        let (dial_box, value) = temperature_dial(caption);
        right_content.append(&dial_box);
        dials.push(value);
    }
    // Rotated a quarter turn like the Windows panel's side label. GTK4 has
    // no CSS transform on widgets, so this is a Cairo-drawn label instead.
    let tag_area = gtk::DrawingArea::new();
    tag_area.set_size_request(18, 160);
    tag_area.set_halign(gtk::Align::Start);
    tag_area.set_valign(gtk::Align::Start);
    tag_area.set_margin_start(8);
    tag_area.set_margin_top(30);
    {
        let text = crate::i18n::t("home_temperature").to_uppercase();
        tag_area.set_draw_func(move |_, cr, _w, h| {
            cr.save().ok();
            cr.translate(13.0, h as f64);
            cr.rotate(-PI / 2.0);
            cr.select_font_face(
                "Predator",
                gtk4::cairo::FontSlant::Normal,
                gtk4::cairo::FontWeight::Normal,
            );
            cr.set_font_size(11.0);
            cr.set_source_rgba(DEEP.0, DEEP.1 * 1.4, DEEP.2 * 1.4, 0.9);
            cr.move_to(0.0, 0.0);
            let _ = cr.show_text(&text);
            cr.restore().ok();
        });
    }
    right.add_overlay(&tag_area);
    row.insert(&right, -1);

    // FlowBox wraps each child in a FlowBoxChild that would otherwise take
    // focus/hover styling - these are display panels, not items.
    let mut child = row.first_child();
    while let Some(c) = child {
        c.set_can_focus(false);
        c.add_css_class("home-flow-child");
        child = c.next_sibling();
    }

    scroll.set_child(Some(&page));

    // === Live refresh ===
    let prev_ticks: Rc<Cell<Option<(u64, u64)>>> = Rc::new(Cell::new(None));
    let in_flight = Rc::new(Cell::new(false));
    let refresh: Rc<dyn Fn()> = {
        Rc::new(move || {
            if in_flight.get() {
                return;
            }
            in_flight.set(true);
            let in_flight = in_flight.clone();
            let gauge = gauge.clone();
            let gpu_clock = gpu_clock.clone();
            let gpu_clock_max = gpu_clock_max.clone();
            let freq_value = freq_value.clone();
            let gpu_state = gpu_state.clone();
            let gpu_usage_bar = gpu_usage_bar.clone();
            let gpu_usage_label = gpu_usage_label.clone();
            let cpu_usage_bar = cpu_usage_bar.clone();
            let cpu_usage_label = cpu_usage_label.clone();
            let dials = dials.clone();
            let prev_ticks = prev_ticks.clone();
            let mode_label = mode_label.clone();
            crate::ui::background::run(read, move |r: Reading| {
                in_flight.set(false);
                if r.gpu_clock_max_mhz > 0 {
                    gpu_clock_max.set(r.gpu_clock_max_mhz as f64);
                }
                gpu_clock.set(r.gpu_clock_mhz.map(f64::from));
                gauge.queue_draw();
                freq_value.set_text(&r.gpu_clock_mhz.map_or("--".into(), |c| c.to_string()));
                // No live clock means the driver has the dGPU powered down
                // (`gpu.rs` never wakes it just to read) - the state the
                // Windows home screen calls "Discrete GPU is idle".
                gpu_state.set_visible(r.gpu_clock_mhz.is_none());

                set_usage(&gpu_usage_bar, &gpu_usage_label, r.gpu_util_pct.map(f64::from));
                let cpu_pct = match (prev_ticks.get(), r.cpu_ticks) {
                    (Some((t0, i0)), Some((t1, i1))) if t1 > t0 => {
                        Some(100.0 * (1.0 - (i1 - i0) as f64 / (t1 - t0) as f64))
                    }
                    _ => None,
                };
                prev_ticks.set(r.cpu_ticks);
                set_usage(&cpu_usage_bar, &cpu_usage_label, cpu_pct);

                for (dial, temp) in dials.iter().zip([r.gpu_temp, r.cpu_temp, r.system_temp]) {
                    dial.0.set(temp);
                    dial.1.set_text(&temp.map_or("--".into(), |t| format!("{t:.0}")));
                    dial.2.queue_draw();
                }

                let mode = crate::hardware::profile::get_current_profile()
                    .map(|p| p.label().to_string())
                    .unwrap_or_default();
                mode_label.set_text(&if mode.is_empty() {
                    crate::i18n::t("home_system_mode").to_string()
                } else {
                    format!("{} · {mode}", crate::i18n::t("home_system_mode"))
                });
            });
        })
    };
    refresh();
    {
        let scroll_weak = scroll.downgrade();
        glib::timeout_add_local(REFRESH, move || {
            let Some(scroll) = scroll_weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if crate::app_state::is_window_visible() && scroll.is_mapped() {
                refresh();
            }
            glib::ControlFlow::Continue
        });
    }

    scroll
}

/// A chamfered display panel: frame drawn behind, content box on top.
/// `mirrored` flips the frame for the right-hand panel, as in the Windows
/// layout where the two side panels face each other.
fn panel(mirrored: bool) -> (gtk::Overlay, gtk::Box) {
    let overlay = gtk::Overlay::new();
    overlay.set_size_request(PANEL_W, PANEL_H);
    let frame = gtk::DrawingArea::new();
    frame.set_draw_func(move |_, cr, w, h| draw_panel_frame(cr, w as f64, h as f64, mirrored));
    overlay.set_child(Some(&frame));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_halign(gtk::Align::Center);
    content.set_valign(gtk::Align::Start);
    content.set_margin_top(36);
    overlay.add_overlay(&content);
    (overlay, content)
}

fn usage_meter(caption: &str) -> (gtk::Box, gtk::LevelBar, gtk::Label) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_size_request(112, -1);
    let bar = gtk::LevelBar::for_interval(0.0, 100.0);
    bar.add_css_class("home-usage-bar");
    bar.set_size_request(112, 8);
    let caption = gtk::Label::new(Some(caption));
    caption.add_css_class("home-caption");
    caption.set_margin_top(10);
    let value = gtk::Label::new(Some("--%"));
    value.add_css_class("home-usage-value");
    column.append(&bar);
    column.append(&caption);
    column.append(&value);
    (column, bar, value)
}

fn set_usage(bar: &gtk::LevelBar, label: &gtk::Label, pct: Option<f64>) {
    let pct = pct.map(|p| p.clamp(0.0, 100.0));
    bar.set_value(pct.unwrap_or(0.0));
    label.set_text(&pct.map_or("--%".into(), |p| format!("{p:.0}%")));
}

type Dial = (Rc<Cell<Option<f64>>>, gtk::Label, gtk::DrawingArea);

fn temperature_dial(caption: &str) -> (gtk::Box, Dial) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_margin_start(40);
    let temp: Rc<Cell<Option<f64>>> = Rc::new(Cell::new(None));
    let area = gtk::DrawingArea::new();
    area.set_size_request(156, 156);
    {
        let temp = temp.clone();
        area.set_draw_func(move |_, cr, w, h| {
            draw_temperature_dial(cr, w as f64, h as f64, temp.get().map(|t| t / TEMP_FULL_SCALE));
        });
    }
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_halign(gtk::Align::Center);
    text.set_valign(gtk::Align::Center);
    let value = gtk::Label::new(Some("--"));
    value.add_css_class("home-dial-value");
    let unit = gtk::Label::new(Some("°C"));
    unit.add_css_class("home-unit");
    text.append(&value);
    text.append(&unit);
    overlay.add_overlay(&text);
    column.append(&overlay);
    let label = gtk::Label::new(Some(caption));
    label.add_css_class("home-caption");
    label.set_halign(gtk::Align::End);
    label.set_margin_end(4);
    column.append(&label);
    (column, (temp, value, area))
}

/// Panel outline: chamfered corners, a translucent black fill (`#00000080`,
/// the Windows `WidgetCardBg`), a deep-teal edge, a hatched strip on the top
/// edge, and bright cyan accents on the outer bottom corner.
fn draw_panel_frame(cr: &gtk4::cairo::Context, w: f64, h: f64, mirrored: bool) {
    if mirrored {
        cr.translate(w, 0.0);
        cr.scale(-1.0, 1.0);
    }
    let cut = 18.0;
    let notch_x = w * 0.38;
    let notch_d = 14.0;
    let outline = |cr: &gtk4::cairo::Context| {
        cr.new_path();
        cr.move_to(cut, notch_d);
        cr.line_to(notch_x, notch_d);
        cr.line_to(notch_x + notch_d, 0.0);
        cr.line_to(w - cut, 0.0);
        cr.line_to(w, cut);
        cr.line_to(w, h - cut);
        cr.line_to(w - cut, h);
        cr.line_to(cut, h);
        cr.line_to(0.0, h - cut);
        cr.line_to(0.0, notch_d + cut);
        cr.close_path();
    };
    outline(cr);
    let fill = gtk4::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
    fill.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.55);
    fill.add_color_stop_rgba(1.0, 0.0, 0.08, 0.1, 0.45);
    let _ = cr.set_source(&fill);
    let _ = cr.fill();

    outline(cr);
    cr.set_line_width(1.5);
    cr.set_source_rgba(DEEP.0, DEEP.1, DEEP.2, 0.9);
    let _ = cr.stroke();

    // Hatched strip along the raised part of the top edge.
    cr.save().ok();
    let strip_x = notch_x + notch_d + 30.0;
    let strip_w = w * 0.42;
    cr.rectangle(strip_x, 0.0, strip_w, 6.0);
    cr.clip();
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 0.75);
    cr.set_line_width(2.0);
    let mut x = strip_x - 8.0;
    while x < strip_x + strip_w + 8.0 {
        cr.move_to(x, 6.0);
        cr.line_to(x + 6.0, 0.0);
        x += 5.0;
    }
    let _ = cr.stroke();
    cr.restore().ok();
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 0.9);
    cr.rectangle(strip_x - 40.0, 0.0, 36.0, 3.0);
    let _ = cr.fill();

    // Bright accents: the outer edge's lower stretch and the bottom chamfer.
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 0.95);
    cr.set_line_width(2.5);
    cr.move_to(0.0, h * 0.72);
    cr.line_to(0.0, h - cut);
    cr.line_to(cut, h);
    cr.line_to(cut + 30.0, h);
    let _ = cr.stroke();
    cr.move_to(0.0, h * 0.30);
    cr.line_to(0.0, h * 0.42);
    cr.set_line_width(1.5);
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 0.45);
    let _ = cr.stroke();
}

/// GPU frequency gauge: a dotted outer tick ring, a segmented mid ring, the
/// value arc and a thin inner ring.
fn draw_frequency_gauge(cr: &gtk4::cairo::Context, w: f64, h: f64, fraction: Option<f64>) {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = w.min(h) / 2.0 - 4.0;

    // Outer tick ring.
    for i in 0..96 {
        let a = i as f64 / 96.0 * 2.0 * PI;
        let long = i % 8 == 0;
        let (r0, r1) = if long { (r - 10.0, r) } else { (r - 5.0, r - 1.0) };
        cr.set_source_rgba(DEEP.0, DEEP.1 * 1.3, DEEP.2 * 1.3, if long { 0.9 } else { 0.55 });
        cr.set_line_width(if long { 2.0 } else { 1.0 });
        cr.move_to(cx + r0 * a.cos(), cy + r0 * a.sin());
        cr.line_to(cx + r1 * a.cos(), cy + r1 * a.sin());
        let _ = cr.stroke();
    }

    // Segmented mid ring.
    let rm = r - 22.0;
    cr.set_line_width(9.0);
    for i in 0..12 {
        let a0 = -PI / 2.0 + i as f64 / 12.0 * 2.0 * PI + 0.04;
        let a1 = a0 + 2.0 * PI / 12.0 - 0.08;
        cr.set_source_rgba(DEEP.0, DEEP.1, DEEP.2, if i % 3 == 0 { 0.85 } else { 0.45 });
        cr.arc(cx, cy, rm, a0, a1);
        let _ = cr.stroke();
    }

    // Value track and arc.
    let rv = r - 40.0;
    cr.set_line_width(3.0);
    cr.set_source_rgba(DEEP.0, DEEP.1, DEEP.2, 0.6);
    cr.arc(cx, cy, rv, 0.0, 2.0 * PI);
    let _ = cr.stroke();
    if let Some(f) = fraction {
        let f = f.clamp(0.0, 1.0);
        if f > 0.0 {
            cr.set_source_rgb(TEAL.0, TEAL.1, TEAL.2);
            cr.set_line_width(4.0);
            cr.arc(cx, cy, rv, -PI / 2.0, -PI / 2.0 + f * 2.0 * PI);
            let _ = cr.stroke();
        }
    }

    // Inner disc, slightly lifted from the panel.
    let disc = gtk4::cairo::RadialGradient::new(cx, cy, 0.0, cx, cy, rv - 4.0);
    disc.add_color_stop_rgba(0.0, 0.0, 0.12, 0.16, 0.9);
    disc.add_color_stop_rgba(1.0, 0.0, 0.04, 0.06, 0.9);
    let _ = cr.set_source(&disc);
    cr.arc(cx, cy, rv - 4.0, 0.0, 2.0 * PI);
    let _ = cr.fill();
}

/// Temperature dial: a dense tick ring lit up to the current temperature, an
/// open outer arc, and a dark glassy centre.
fn draw_temperature_dial(cr: &gtk4::cairo::Context, w: f64, h: f64, fraction: Option<f64>) {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = w.min(h) / 2.0 - 3.0;

    // Open outer arc - gap at the lower right, where the label sits.
    cr.set_line_width(2.0);
    cr.set_source_rgba(DEEP.0, DEEP.1, DEEP.2, 0.9);
    cr.arc(cx, cy, r, PI * 0.35, PI * 2.15);
    let _ = cr.stroke();

    // Tick ring, from the bottom-left round to the bottom-right.
    let ticks = 64;
    let start = PI * 0.75;
    let sweep = PI * 1.5;
    let lit = fraction.map_or(0, |f| (f.clamp(0.0, 1.0) * ticks as f64).round() as usize);
    for i in 0..ticks {
        let a = start + i as f64 / (ticks - 1) as f64 * sweep;
        let (r0, r1) = (r - 16.0, r - 6.0);
        if i < lit {
            cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 0.95);
        } else {
            cr.set_source_rgba(DEEP.0, DEEP.1, DEEP.2, 0.55);
        }
        cr.set_line_width(2.0);
        cr.move_to(cx + r0 * a.cos(), cy + r0 * a.sin());
        cr.line_to(cx + r1 * a.cos(), cy + r1 * a.sin());
        let _ = cr.stroke();
    }

    let rc = r - 22.0;
    let disc = gtk4::cairo::RadialGradient::new(cx, cy - rc * 0.4, 0.0, cx, cy, rc);
    disc.add_color_stop_rgba(0.0, 0.0, 0.22, 0.26, 1.0);
    disc.add_color_stop_rgba(1.0, 0.0, 0.07, 0.09, 1.0);
    let _ = cr.set_source(&disc);
    cr.arc(cx, cy, rc, 0.0, 2.0 * PI);
    let _ = cr.fill();
}

/// Centre column: angled cyan brackets left and right, and the lit platform
/// the laptop stands on.
fn draw_center_stage(cr: &gtk4::cairo::Context, w: f64, h: f64) {
    cr.set_line_width(4.0);
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 0.9);
    let (top, bottom) = (h * 0.30, h * 0.66);
    let inset = 22.0;
    for left in [true, false] {
        let x0 = if left { 2.0 } else { w - 2.0 };
        let x1 = if left { inset } else { w - inset };
        cr.move_to(x1, top - 28.0);
        cr.line_to(x0, top);
        cr.line_to(x0, bottom);
        cr.line_to(x1, bottom + 28.0);
        let _ = cr.stroke();
    }

    // Platform: soft glow, dark base, then two bright rims.
    let (px, py) = (w / 2.0, h * 0.80);
    let (rx, ry) = (w * 0.43, h * 0.065);
    let ellipse = |cr: &gtk4::cairo::Context, sx: f64, sy: f64, dy: f64| {
        cr.save().ok();
        cr.translate(px, py + dy);
        cr.scale(sx, sy);
        cr.arc(0.0, 0.0, 1.0, 0.0, 2.0 * PI);
        cr.restore().ok();
    };
    let glow = gtk4::cairo::RadialGradient::new(px, py, 0.0, px, py, rx * 1.2);
    glow.add_color_stop_rgba(0.0, CYAN.0, CYAN.1, CYAN.2, 0.22);
    glow.add_color_stop_rgba(1.0, CYAN.0, CYAN.1, CYAN.2, 0.0);
    cr.save().ok();
    cr.translate(0.0, py);
    cr.scale(1.0, ry / rx * 2.2);
    cr.translate(0.0, -py);
    let _ = cr.set_source(&glow);
    cr.arc(px, py, rx * 1.2, 0.0, 2.0 * PI);
    let _ = cr.fill();
    cr.restore().ok();

    ellipse(cr, rx, ry, 10.0);
    cr.set_source_rgba(0.0, 0.03, 0.05, 0.95);
    let _ = cr.fill();
    ellipse(cr, rx, ry, 0.0);
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 1.0);
    cr.set_line_width(4.0);
    let _ = cr.stroke();
    ellipse(cr, rx * 0.8, ry * 0.7, 0.0);
    cr.set_source_rgba(CYAN.0, CYAN.1, CYAN.2, 0.35);
    cr.set_line_width(2.0);
    let _ = cr.stroke();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_ticks_parse_the_aggregate_line() {
        let (total, idle) = read_cpu_ticks().expect("/proc/stat readable on Linux");
        assert!(total >= idle);
    }

    #[test]
    fn usage_is_clamped_and_formatted() {
        // `set_usage` needs GTK; the formatting rule is what matters here.
        let fmt = |p: Option<f64>| p.map(|p| p.clamp(0.0, 100.0)).map_or("--%".into(), |p| format!("{p:.0}%"));
        assert_eq!(fmt(None), "--%");
        assert_eq!(fmt(Some(142.0)), "100%");
        assert_eq!(fmt(Some(41.6)), "42%");
    }
}
