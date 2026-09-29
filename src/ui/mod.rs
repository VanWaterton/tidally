mod theme;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Margin, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Cell, Clear, Padding, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState, Wrap,
};
use ratatui_image::Image;

use crate::app::{App, Auth, Focus, Hits, Level, View};
use crate::art::Palette;
use crate::audio::Route;
use crate::eq;
use crate::search;
use crate::tidal::models::Track;
use theme::*;

pub use theme::{DEFAULT_ACCENT, DEFAULT_SECONDARY};

/// Terminals narrower than this don't get the art/visualizer sidebar.
const SIDEBAR_MIN_WIDTH: u16 = 110;

pub fn set_palette(p: &Palette) {
    theme::set_palette(p.accent, p.secondary);
}

pub fn reset_palette() {
    theme::set_palette(DEFAULT_ACCENT, DEFAULT_SECONDARY);
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.art.wanted = None;
    app.hits = Hits::default();
    let [header, body, player, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(4),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, app, header);
    if app.view == View::NowPlaying {
        draw_now_playing(f, app, body);
    } else {
        let main = if body.width >= SIDEBAR_MIN_WIDTH {
            let side_w = (body.width * 3 / 10).clamp(34, 52);
            let [main, side] =
                Layout::horizontal([Constraint::Min(60), Constraint::Length(side_w)]).areas(body);
            draw_sidebar(f, app, side);
            main
        } else {
            body
        };
        match app.view {
            View::Search => draw_search(f, app, main),
            _ => draw_queue(f, app, main),
        }
    }
    draw_player(f, app, player);
    draw_footer(f, app, footer);

    match &app.auth {
        Auth::Ready => {}
        auth => draw_login(f, auth, app.frame),
    }
    if app.show_help {
        draw_help(f);
    }
}

// ── header ──────────────────────────────────────────────────────────────────

fn draw_header(f: &mut Frame, app: &mut App, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    // Logo in the album gradient.
    let logo = "◆ TIDALLY";
    let n = logo.chars().count().max(2) - 1;
    for (i, c) in logo.chars().enumerate() {
        spans.push(Span::styled(
            c.to_string(),
            Style::new().fg(gradient(i as f32 / n as f32)).bold(),
        ));
    }
    spans.push(Span::raw("   "));

    let queue_label = match app.queue.tracks.len() {
        0 => "Queue".to_string(),
        n => format!("Queue {n}"),
    };
    for (key, label, view) in [
        ("1", "Search".to_string(), View::Search),
        ("2", queue_label, View::Queue),
        ("3", "Now Playing".to_string(), View::NowPlaying),
    ] {
        let x = area.x + Line::from(spans.clone()).width() as u16;
        let tab_width = Span::raw(format!(" {key} {label} ")).width() as u16;
        app.hits
            .tabs
            .push((Rect::new(x, area.y, tab_width, 1), view));
        if app.view == view {
            let pill = Style::new().fg(BASE).bg(accent_color()).bold();
            spans.push(Span::styled(format!(" {key} {label} "), pill));
        } else {
            spans.push(Span::styled(format!(" {key} "), faint()));
            spans.push(Span::styled(format!("{label} "), muted()));
        }
        spans.push(Span::raw(" "));
    }
    f.render_widget(Line::from(spans), area);

    let preset = &eq::PRESETS[app.eq];
    let route = match &app.route {
        #[cfg(feature = "remote")]
        Route::Pulse(_) => Span::styled("⇄ remote audio   ", secondary().bold()),
        #[cfg(feature = "remote")]
        Route::LocalOverSsh => Span::styled("host speakers   ", Style::new().fg(ERROR)),
        #[cfg(feature = "remote")]
        Route::Unreachable(_) => Span::styled("✗ remote audio   ", Style::new().fg(ERROR).bold()),
        Route::Local => Span::raw(""),
    };
    let route_width = route.width() as u16;
    let eq_spans = [
        Span::styled("EQ ", faint()),
        Span::styled(format!("{} ", preset.name), secondary().bold()),
    ];
    let eq_width = eq_spans.iter().map(Span::width).sum::<usize>() as u16;
    let [eq_label, eq_name] = eq_spans;
    let right = Line::from(vec![
        route,
        eq_label,
        eq_name,
        Span::styled(" ", Style::new()),
        Span::styled(
            // Show what Tidal actually served, which can be lower than what was requested.
            format!(
                " {} ",
                app.now
                    .stream
                    .as_ref()
                    .and_then(|s| s.quality.as_deref())
                    .unwrap_or(&app.quality)
            ),
            Style::new().fg(BASE).bg(BADGE).bold(),
        ),
        Span::raw(" "),
    ])
    .right_aligned();
    let start = area.right().saturating_sub(right.width() as u16);
    app.hits.eq = Some(Rect::new(start + route_width, area.y, eq_width, 1));
    f.render_widget(right, area);
}

// ── search / queue ──────────────────────────────────────────────────────────

fn draw_search(f: &mut Frame, app: &mut App, area: Rect) {
    let [input_area, results_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);

    let focused = app.focus == Focus::Input;
    let input_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border(focused))
        .title(Line::from(Span::styled(
            " Search ",
            if focused { bold_accent() } else { muted() },
        )));

    const PROMPT: &str = " › ";
    let mut content = vec![Span::styled(PROMPT, bold_accent())];
    if app.search.input.is_empty() {
        content.push(Span::styled(
            "Search…   or: artist <name>  album <name>  track <name>",
            faint().italic(),
        ));
    } else {
        content.extend(
            search::highlight(&app.search.input)
                .into_iter()
                .map(|(run, keyword)| {
                    Span::styled(run, if keyword { secondary().bold() } else { text() })
                }),
        );
    }
    let inner = input_block.inner(input_area);
    app.hits.search_input = Some(input_area);
    f.render_widget(
        Paragraph::new(Line::from(content)).block(input_block),
        input_area,
    );
    if focused && matches!(app.auth, Auth::Ready) {
        let before: String = app.search.input.chars().take(app.search.cursor).collect();
        let offset = Span::raw(PROMPT).width() + Span::raw(before).width();
        let x = inner.x.saturating_add(offset as u16);
        f.set_cursor_position((x.min(inner.right().saturating_sub(1)), inner.y));
    }

    let title = if app.search.loading {
        Line::from(vec![
            Span::styled(format!(" {} ", spinner(app.frame)), accent()),
            Span::styled(format!("Searching “{}” ", app.search.last_query), muted()),
        ])
    } else if app.search.results.is_empty() {
        Line::styled(" Results ", muted())
    } else {
        Line::from(vec![
            Span::styled(format!(" {} ", app.search.heading), bold_accent()),
            Span::styled(format!("· {} tracks ", app.search.results.len()), muted()),
        ])
    };

    let playing_id = app.queue.current_track().map(|t| t.id);
    let block = list_block(title, app.focus == Focus::List);
    if app.search.results.is_empty() && !app.search.loading {
        let example = |kw: &'static str, rest: &'static str, what: &'static str| {
            Line::from(vec![
                Span::styled(kw, secondary().bold()),
                Span::styled(rest, text()),
                Span::styled(format!("  {what}"), faint()),
            ])
        };
        let hint = Text::from(vec![
            Line::styled("♪", bold_accent()),
            Line::raw(""),
            Line::styled("Type a query and press Enter", muted()),
            Line::raw(""),
            example("artist", " radiohead", "top tracks"),
            example("album", " ok computer", "whole album"),
            Line::from(vec![
                Span::styled("artist", secondary().bold()),
                Span::styled(" radiohead ", text()),
                Span::styled("album", secondary().bold()),
                Span::styled(" kid a", text()),
                Span::styled("  that artist's album", faint()),
            ]),
            example("track", " creep", "filter by any fields"),
        ])
        .alignment(Alignment::Center);
        let inner = block.inner(results_area);
        f.render_widget(block, results_area);
        let [centered] = Layout::vertical([Constraint::Length(8)])
            .flex(Flex::Center)
            .areas(inner);
        f.render_widget(Paragraph::new(hint), centered);
        return;
    }
    let rows = render_tracks(
        f,
        results_area,
        block,
        &app.search.results,
        &mut app.search.table,
        playing_id,
        None,
    );
    app.hits.list_rows = Some(rows);
}

fn draw_queue(f: &mut Frame, app: &mut App, area: Rect) {
    let total = app.queue.total_secs();
    let title = Line::from(vec![
        Span::styled(" Queue ", bold_accent()),
        Span::styled(
            format!(
                "· {} tracks · {} ",
                app.queue.tracks.len(),
                format_long(total)
            ),
            muted(),
        ),
    ]);
    let block = list_block(title, true);
    if app.queue.tracks.is_empty() {
        let hint = Paragraph::new(vec![
            Line::raw(""),
            Line::styled("Queue is empty", muted()),
            Line::styled("Press a on a search result to add it", faint()),
        ])
        .alignment(Alignment::Center)
        .block(block);
        f.render_widget(hint, area);
        return;
    }
    let playing_id = app.queue.current_track().map(|t| t.id);
    let rows = render_tracks(
        f,
        area,
        block,
        &app.queue.tracks,
        &mut app.queue.table,
        playing_id,
        app.queue.current,
    );
    app.hits.list_rows = Some(rows);
}

fn list_block(title: Line<'_>, focused: bool) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border(focused))
        .title(title)
        .padding(Padding::horizontal(1))
}

fn render_tracks(
    f: &mut Frame,
    area: Rect,
    block: Block,
    tracks: &[Track],
    state: &mut TableState,
    playing_id: Option<u64>,
    playing_index: Option<usize>,
) -> Rect {
    let header_style = Style::new().fg(accent_dim()).bold();
    let header = Row::new(
        ["", "#", "Title", "Artist", "Album", "Time"].map(|h| Cell::from(h).style(header_style)),
    );

    let artist_color = lerp(MUTED, accent_color(), 0.55);
    let album_color = lerp(FAINT, secondary_color(), 0.55);
    let rows = tracks.iter().enumerate().map(|(i, t)| {
        // In the queue, only the exact queued position is marked; in search, match by id.
        let playing = match playing_index {
            Some(idx) => idx == i,
            None => playing_id == Some(t.id),
        };
        let marker = if playing {
            Span::styled("♪", secondary().bold())
        } else {
            Span::raw("")
        };
        let title_style = if playing { bold_accent() } else { text() };
        let mut title = vec![Span::styled(t.display_title(), title_style)];
        if t.explicit {
            title.push(Span::styled(" E", faint().bold()));
        }
        Row::new(vec![
            Cell::from(marker),
            Cell::from(Span::styled(format!("{}", i + 1), faint())),
            Cell::from(Line::from(title)),
            Cell::from(Span::styled(
                t.artist_names(),
                Style::new().fg(artist_color),
            )),
            Cell::from(Span::styled(
                t.album_title().to_string(),
                Style::new().fg(album_color),
            )),
            Cell::from(Line::styled(format_time(t.duration as f64), muted()).right_aligned()),
        ])
    });

    let widths = [
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Fill(4),
        Constraint::Fill(3),
        Constraint::Fill(3),
        Constraint::Length(5),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(2)
        .row_highlight_style(selected_row())
        .highlight_symbol(Span::styled("▌", bold_accent()))
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always);
    f.render_stateful_widget(table, area, state);

    let visible = area.height.saturating_sub(3) as usize;
    if tracks.len() > visible {
        let mut sb =
            ScrollbarState::new(tracks.len().saturating_sub(visible)).position(state.offset());
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(Style::new().fg(BORDER))
                .thumb_style(accent()),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut sb,
        );
    }
    // Data rows sit inside the border, below the header row.
    Rect::new(
        area.x + 1,
        area.y + 2,
        area.width.saturating_sub(2),
        area.height.saturating_sub(3),
    )
}

// ── art, spectrum, eq ───────────────────────────────────────────────────────

/// Draws the cover as large as fits in `area` (square, centered). Returns the rect used.
fn draw_art(f: &mut Frame, app: &mut App, area: Rect) -> Rect {
    let aspect = app.art.cell_aspect();
    // A square image is `h` rows tall and `h * aspect` columns wide.
    let h = area.height.min((area.width as f32 / aspect) as u16);
    let w = ((h as f32 * aspect).round() as u16).min(area.width);
    let [v] = Layout::vertical([Constraint::Length(h)])
        .flex(Flex::Start)
        .areas(area);
    let [rect] = Layout::horizontal([Constraint::Length(w)])
        .flex(Flex::Center)
        .areas(v);

    if app.art.cover.is_some() {
        app.art.wanted = Some(Size::new(rect.width, rect.height));
    }
    match &app.art.protocol {
        Some((_, protocol)) if app.art.cover.is_some() => {
            // Center the encoded image (fit may be narrower or shorter than the rect).
            let size = protocol.size();
            let x = rect.x + rect.width.saturating_sub(size.width) / 2;
            let y = rect.y + rect.height.saturating_sub(size.height) / 2;
            let img_rect = Rect::new(
                x,
                y,
                size.width.min(rect.width),
                size.height.min(rect.height),
            );
            f.render_widget(Image::new(protocol), img_rect);
        }
        _ => draw_art_placeholder(f, rect, app.art.cover.is_some(), app.frame),
    }
    rect
}

/// A soft accent→secondary gradient tile with a note, shown while loading or with no cover.
fn draw_art_placeholder(f: &mut Frame, rect: Rect, loading: bool, frame: usize) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let buf = f.buffer_mut();
    let strength = if loading { 0.35 } else { 0.18 };
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            let t = ((x - rect.x) as f32 / rect.width as f32
                + (y - rect.y) as f32 / rect.height as f32)
                / 2.0;
            let bg = lerp(BASE, gradient(t), strength);
            buf[(x, y)].set_symbol(" ").set_bg(bg);
        }
    }
    let glyph = if loading { spinner(frame) } else { "♪" };
    let cx = rect.x + rect.width / 2;
    let cy = rect.y + rect.height / 2;
    buf[(cx, cy)]
        .set_symbol(glyph)
        .set_fg(TEXT)
        .set_style(Style::new().add_modifier(Modifier::BOLD));
}

const BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Vertical spectrum bars with an accent (bottom) → secondary (top) gradient.
fn draw_spectrum(f: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if let Some(err) = app.visualizer.error() {
        f.render_widget(
            Paragraph::new(Line::styled(format!("visualizer off: {err}"), faint()))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let active = app.queue.current.is_some();
    let bands = app.visualizer.bands();

    let gap = 1u16;
    let count = ((area.width + gap) / (1 + gap))
        .min(bands.len() as u16)
        .max(1);
    let bar_w = ((area.width + gap) / count).saturating_sub(gap).max(1);
    let used = count * (bar_w + gap) - gap;
    let x0 = area.x + area.width.saturating_sub(used) / 2;

    let buf = f.buffer_mut();
    let rows = area.height as usize;
    for i in 0..count as usize {
        let lo = i * bands.len() / count as usize;
        let hi = ((i + 1) * bands.len() / count as usize).max(lo + 1);
        let level = if active {
            bands[lo..hi].iter().copied().fold(0.0, f32::max)
        } else {
            0.0
        };
        let eighths = (level * (rows * 8) as f32).round() as usize;
        for r in 0..rows {
            let fill = eighths.saturating_sub(r * 8).min(8);
            // Always show a faint floor so the visualizer reads as a shape even when silent.
            let (sym, color) = if fill == 0 && r == 0 {
                ("▁", BORDER)
            } else {
                (BLOCKS[fill], gradient(r as f32 / rows.max(2) as f32 * 1.1))
            };
            let y = area.bottom() - 1 - r as u16;
            for dx in 0..bar_w {
                let x = x0 + i as u16 * (bar_w + gap) + dx;
                if x < area.right() {
                    buf[(x, y)].set_symbol(sym).set_fg(color);
                }
            }
        }
    }
}

/// EQ preset curve. Tall areas get bars around a 0dB line with labels; short ones a sparkline.
fn draw_eq(f: &mut Frame, eq_index: usize, area: Rect) {
    let preset = &eq::PRESETS[eq_index];
    if area.height < 5 {
        let mut spans = vec![
            Span::styled("EQ ", faint()),
            Span::styled(format!("{:<12}", preset.name), secondary().bold()),
        ];
        for (i, g) in preset.gains.iter().enumerate() {
            let level = (((g + 6.0) / 12.0) * 7.0).round().clamp(0.0, 7.0) as usize + 1;
            spans.push(Span::styled(
                BLOCKS[level],
                Style::new().fg(gradient(i as f32 / 9.0)),
            ));
        }
        spans.push(Span::styled("  e", accent()));
        f.render_widget(Line::from(spans), area);
        return;
    }

    let [title, graph, labels] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    f.render_widget(
        Line::from(vec![
            Span::styled("Equalizer  ", muted()),
            Span::styled(preset.name, secondary().bold()),
            Span::styled("   e / E to change", faint()),
        ]),
        title,
    );

    let col_w = (graph.width / eq::FREQS.len() as u16).max(2);
    let bar_w = (col_w / 2).max(1);
    let mid = graph.y + graph.height / 2;
    let half = (graph.height / 2).max(1) as f32;
    for x in graph.left()..graph.right() {
        f.buffer_mut()[(x, mid)].set_symbol("┈").set_fg(BORDER);
    }
    for (i, g) in preset.gains.iter().enumerate() {
        let color = gradient(i as f32 / 9.0);
        let cells = ((g.abs() / 6.0) * half).round() as u16;
        let x0 = graph.x + i as u16 * col_w + (col_w - bar_w) / 2;
        for dx in 0..bar_w {
            let x = x0 + dx;
            if x >= graph.right() {
                continue;
            }
            if cells == 0 {
                f.buffer_mut()[(x, mid)].set_symbol("━").set_fg(color);
            }
            for c in 0..cells {
                let y = if *g > 0.0 {
                    mid.saturating_sub(c + 1)
                } else {
                    mid + c + 1
                };
                if y >= graph.top() && y < graph.bottom() {
                    f.buffer_mut()[(x, y)].set_symbol("█").set_fg(color);
                }
            }
        }
        let label = match eq::FREQS[i] {
            f if f >= 1000 => format!("{}k", f / 1000),
            f => f.to_string(),
        };
        let lx = graph.x + i as u16 * col_w;
        let lrect = Rect::new(
            lx,
            labels.y,
            col_w.min(labels.right().saturating_sub(lx)),
            1,
        );
        f.render_widget(Line::styled(label, faint()).centered(), lrect);
    }
}

fn draw_track_info(f: &mut Frame, app: &App, area: Rect, centered: bool) {
    let Some(track) = app.queue.current_track() else {
        f.render_widget(
            Paragraph::new(Line::styled("Nothing playing", faint())).alignment(if centered {
                Alignment::Center
            } else {
                Alignment::Left
            }),
            area,
        );
        return;
    };
    let mut lines = vec![
        Line::styled(track.display_title(), text().bold()),
        Line::styled(track.artist_names(), accent()),
        Line::styled(
            track.album_title().to_string(),
            Style::new().fg(lerp(MUTED, secondary_color(), 0.6)),
        ),
    ];
    if area.height > 3 {
        let mut meta = vec![];
        if let Some(stream) = &app.now.stream {
            meta.push(Span::styled(
                format!(" {} ", stream.badge()),
                Style::new().fg(BASE).bg(BADGE).bold(),
            ));
            meta.push(Span::raw("  "));
        }
        if let Some(i) = app.queue.current {
            meta.push(Span::styled(
                format!("{} of {}", i + 1, app.queue.tracks.len()),
                faint(),
            ));
        }
        lines.push(Line::from(meta));
    }
    let align = if centered {
        Alignment::Center
    } else {
        Alignment::Left
    };
    f.render_widget(Paragraph::new(lines).alignment(align), area);
}

fn draw_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border(false))
        .title(Line::styled(" Now Playing ", muted()))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Cover gets up to half the height; info, spectrum and EQ share the rest.
    let art_h = ((inner.width as f32 / app.art.cell_aspect()) as u16)
        .min(inner.height.saturating_sub(9) * 2 / 3);
    let [art, _, info, _, spectrum, eq_line] = Layout::vertical([
        Constraint::Length(art_h),
        Constraint::Length(1),
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(inner);
    if art_h > 0 {
        draw_art(f, app, art);
    }
    draw_track_info(f, app, info, true);
    draw_spectrum(f, app, spectrum);
    draw_eq(f, app.eq, eq_line);
}

fn draw_now_playing(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border(true))
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let aspect = app.art.cell_aspect();
    let art_w = ((inner.height as f32 * aspect) as u16).min(inner.width / 2);
    let [art, _, right] = Layout::horizontal([
        Constraint::Length(art_w),
        Constraint::Length(3),
        Constraint::Min(20),
    ])
    .areas(inner);
    draw_art(f, app, art);

    let eq_h = if right.height >= 20 { 9 } else { 1 };
    let [info, _, spectrum, _, eq_area] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(1),
        Constraint::Length(eq_h),
    ])
    .areas(right);
    draw_track_info(f, app, info, false);
    draw_spectrum(f, app, spectrum);
    draw_eq(f, app.eq, eq_area);
}

// ── player bar & footer ─────────────────────────────────────────────────────

fn draw_player(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(lerp(BORDER, accent_color(), 0.3)))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [info, progress] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);

    // Touch-friendly transport buttons: ⏮ ▶/⏸ ⏭, each a 3-cell tinted pill.
    let track = app.queue.current_track();
    let button_bg = lerp(
        BASE,
        accent_color(),
        if track.is_some() { 0.28 } else { 0.1 },
    );
    let button =
        |glyph: &'static str, style: Style| Span::styled(format!(" {glyph} "), style.bg(button_bg));
    let play_glyph = if app.now.loading {
        spinner(app.frame)
    } else if app.now.paused || track.is_none() {
        "▶"
    } else {
        "⏸"
    };
    let mut left = vec![
        button("⏮", text()),
        Span::raw(" "),
        button(play_glyph, bold_accent()),
        Span::raw(" "),
        button("⏭", text()),
        Span::raw("  "),
    ];
    app.hits.prev = Some(Rect::new(info.x, info.y, 3, 1));
    app.hits.play = Some(Rect::new(info.x + 4, info.y, 3, 1));
    app.hits.next = Some(Rect::new(info.x + 8, info.y, 3, 1));

    let Some(track) = track else {
        left.push(Span::styled("Nothing playing", faint()));
        f.render_widget(Line::from(left), info);
        app.hits.progress = draw_progress(f, progress, 0.0, 0.0, app.now.volume);
        return;
    };
    left.extend([
        Span::styled(track.display_title(), text().bold()),
        Span::styled("  ", faint()),
        Span::styled(track.artist_names(), accent()),
    ]);
    if !track.album_title().is_empty() {
        left.push(Span::styled(" · ", faint()));
        left.push(Span::styled(
            track.album_title().to_string(),
            Style::new().fg(lerp(FAINT, secondary_color(), 0.7)),
        ));
    }
    f.render_widget(Line::from(left), info);

    if let Some(stream) = &app.now.stream {
        let badge = Line::styled(
            format!(" {} ", stream.badge()),
            Style::new().fg(BASE).bg(BADGE).bold(),
        )
        .right_aligned();
        f.render_widget(badge, info);
    }

    app.hits.progress = draw_progress(
        f,
        progress,
        app.now.position,
        app.now.duration,
        app.now.volume,
    );
}

/// Draws the progress line and returns the seekable bar's area.
fn draw_progress(
    f: &mut Frame,
    area: Rect,
    position: f64,
    duration: f64,
    volume: f64,
) -> Option<Rect> {
    let elapsed = format_time(position);
    let total = format_time(duration);
    let vol = format!("  {} {:>3.0}%", volume_icon(volume), volume);

    // Measure display width, not chars: the volume emoji is two cells wide.
    let fixed = [&elapsed, &total, &vol]
        .iter()
        .map(|s| Span::raw(s.as_str()).width())
        .sum::<usize>()
        + 2;
    let bar_width = (area.width as usize).saturating_sub(fixed);
    let ratio = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((bar_width as f64) * ratio).round() as usize;

    let bar_x = area.x + Span::raw(elapsed.as_str()).width() as u16 + 1;
    let mut spans = vec![Span::styled(elapsed, muted()), Span::raw(" ")];
    if bar_width > 0 {
        let head = filled.min(bar_width.saturating_sub(1));
        // Filled part sweeps accent → secondary across the whole bar width.
        for i in 0..head {
            let color = gradient(i as f32 / bar_width as f32);
            spans.push(Span::styled("━", Style::new().fg(color)));
        }
        let knob = if duration > 0.0 {
            Style::new().fg(TEXT).bold()
        } else {
            faint()
        };
        spans.push(Span::styled("●", knob));
        spans.push(Span::styled(
            "─".repeat(bar_width - head - 1),
            Style::new().fg(BORDER),
        ));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(total, muted()));
    spans.push(Span::styled(vol, muted()));
    f.render_widget(Line::from(spans), area);
    (bar_width > 0).then(|| Rect::new(bar_x, area.y, bar_width as u16, 1))
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    if let Some(status) = &app.status {
        let (icon, color) = match status.level {
            Level::Info => ("●", accent_color()),
            Level::Success => ("✓", SUCCESS),
            Level::Error => ("✗", ERROR),
        };
        let line = Line::from(vec![
            Span::styled(format!(" {icon} "), Style::new().fg(color).bold()),
            Span::styled(status.text.as_str(), Style::new().fg(color)),
        ]);
        f.render_widget(line, area);
        return;
    }

    let hints: &[(&str, &str)] = match (app.view, app.focus) {
        (View::Search, Focus::Input) => &[
            ("⏎", "search"),
            ("esc", "results"),
            ("^u", "clear"),
            ("^c", "quit"),
        ],
        (View::Search, Focus::List) => &[
            ("⏎", "play"),
            ("a", "queue"),
            ("N", "play next"),
            ("/", "search"),
            ("␣", "pause"),
            ("n/p", "next/prev"),
            ("e", "eq"),
            ("?", "help"),
            ("q", "quit"),
        ],
        (View::Queue, _) => &[
            ("⏎", "play"),
            ("d", "remove"),
            ("J/K", "move"),
            ("c", "clear"),
            ("␣", "pause"),
            ("n/p", "next/prev"),
            ("e", "eq"),
            ("?", "help"),
            ("q", "quit"),
        ],
        (View::NowPlaying, _) => &[
            ("␣", "pause"),
            ("n/p", "next/prev"),
            ("←/→", "seek"),
            ("+/-", "volume"),
            ("e/E", "eq preset"),
            ("tab", "view"),
            ("?", "help"),
            ("q", "quit"),
        ],
    };
    f.render_widget(key_hints(hints), area);
}

fn key_hints<'a>(hints: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = vec![Span::raw(" ")];
    let n = hints.len().max(2) - 1;
    for (i, (key, label)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", faint()));
        }
        spans.push(Span::styled(
            *key,
            Style::new().fg(gradient(i as f32 / n as f32)).bold(),
        ));
        spans.push(Span::styled(format!(" {label}"), faint()));
    }
    Line::from(spans)
}

// ── popups ──────────────────────────────────────────────────────────────────

fn draw_login(f: &mut Frame, auth: &Auth, frame: usize) {
    let area = centered(f.area(), 52, 11);
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(accent())
        .title(Line::styled(" Sign in to Tidal ", bold_accent()).centered())
        .padding(Padding::uniform(1));

    let lines = match auth {
        Auth::Starting => vec![
            Line::raw(""),
            Line::from(vec![
                Span::styled(spinner(frame), accent()),
                Span::styled("  Contacting Tidal…", muted()),
            ]),
        ],
        Auth::Pending(code) => vec![
            Line::styled("Visit this link and approve the device:", muted()),
            Line::raw(""),
            Line::styled(code.link(), accent().underlined()),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Code  ", muted()),
                Span::styled(
                    format!(" {} ", code.user_code),
                    Style::new().fg(BASE).bg(accent_color()).bold(),
                ),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::styled(spinner(frame), accent()),
                Span::styled("  waiting…   ", faint()),
                Span::styled("o", accent()),
                Span::styled(" open browser  ", faint()),
                Span::styled("q", accent()),
                Span::styled(" quit", faint()),
            ]),
        ],
        Auth::Failed(err) => vec![
            Line::styled("Sign-in failed", Style::new().fg(ERROR).bold()),
            Line::raw(""),
            Line::styled(err.as_str(), muted()),
            Line::raw(""),
            Line::from(vec![
                Span::styled("r", accent()),
                Span::styled(" retry  ", faint()),
                Span::styled("q", accent()),
                Span::styled(" quit", faint()),
            ]),
        ],
        Auth::Ready => return,
    };
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(block),
        area,
    );
}

fn draw_help(f: &mut Frame) {
    let sections: &[(&str, &[(&str, &str)])] = &[
        (
            "Playback",
            &[
                ("space", "play / pause"),
                ("n / p", "next / previous"),
                ("← → / h l", "seek ±5s"),
                ("+ / -", "volume"),
                ("e / E", "next / previous EQ preset"),
            ],
        ),
        (
            "Navigation",
            &[
                ("/", "focus search"),
                ("tab / 1 2 3", "search · queue · now playing"),
                ("j k / ↑ ↓", "move"),
                ("g / G", "top / bottom"),
            ],
        ),
        (
            "Tracks",
            &[
                ("enter", "play (search: play list from here)"),
                ("a", "add to queue"),
                ("N", "play next"),
                ("d", "remove from queue"),
                ("J / K", "reorder queue"),
                ("c", "clear queue"),
            ],
        ),
    ];

    let mut lines = Vec::new();
    for (i, (title, keys)) in sections.iter().enumerate() {
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        let color: Color = gradient(i as f32 / 2.0);
        lines.push(Line::styled(*title, Style::new().fg(color).bold()));
        for (key, desc) in *keys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<13}"), accent()),
                Span::styled(*desc, muted()),
            ]));
        }
    }

    let height = lines.len() as u16 + 4;
    let area = centered(f.area(), 56, height);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(accent())
                .title(Line::styled(" Keys ", bold_accent()).centered())
                .title_bottom(Line::styled(" any key to close ", faint()).centered())
                .padding(Padding::uniform(1)),
        ),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [h] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(v);
    h
}

fn volume_icon(volume: f64) -> &'static str {
    match volume {
        v if v <= 0.0 => "🔇",
        v if v < 50.0 => "🔉",
        _ => "🔊",
    }
}

fn format_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn format_long(secs: u64) -> String {
    match (secs / 3600, (secs % 3600) / 60) {
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} hr {m} min"),
    }
}
