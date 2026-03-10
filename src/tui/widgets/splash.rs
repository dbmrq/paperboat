//! Splash screen widget for Paperboat TUI.
//!
//! Displays the paperboat ASCII art centered on screen with a warm-toned
//! animated color mesh rippling through the characters.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// ASCII art for the paperboat splash screen.
const PAPERBOAT_ART: &str = r"
                       ....
                     ........
                    ..........
                    .........
                     .......



                                                 ++
                                                +. +
                                              ## #   +
                              ++#+           +   #    +            +#++
                               +   ++------.+    +     .+.----.## ...#-
                                #       ###      + .....-++ ........++
                                 +           .+ .+#+ ...............+
            ----------------------#          ......................+       --------------------
         -------           --++-++##     .........................+     ----------------------
    -----------------          +---#++# .........................++------             -----
          ------            --------------++--+##+++++####++++++++----------------
                ------------------------------+- --------              ----
                                        ---             ---------
";

/// Warm color palette for the ripple animation using ANSI 256-color palette.
/// These medium-toned colors are chosen to have good contrast on both
/// light and dark terminal backgrounds.
const WARM_COLORS: [Color; 6] = [
    Color::Indexed(178), // Goldenrod yellow
    Color::Indexed(172), // Orange
    Color::Indexed(166), // Dark orange
    Color::Indexed(130), // Brown-orange
    Color::Indexed(136), // Dark goldenrod
    Color::Indexed(172), // Orange (cycle back)
];

/// Base color for non-highlighted characters.
/// Using a neutral gray that's visible on both light and dark backgrounds.
const BASE_COLOR: Color = Color::DarkGray;

/// Renders the splash screen with animated warm color ripple.
///
/// The animation creates a diagonal wave pattern that ripples through
/// the ASCII art characters, giving a subtle "mesh" effect with warm tones.
///
/// # Arguments
///
/// * `frame` - The ratatui frame to render to
/// * `area` - The full terminal area
/// * `animation_frame` - Current animation frame for timing the ripple
pub fn render_splash_screen(frame: &mut Frame, area: Rect, animation_frame: u32) {
    let lines: Vec<&str> = PAPERBOAT_ART.lines().collect();
    let art_height = lines.len() as u16;
    let art_width = lines.iter().map(|l| l.len()).max().unwrap_or(0) as u16;

    // Calculate centering offsets
    let vertical_padding = area.height.saturating_sub(art_height) / 2;
    let horizontal_padding = area.width.saturating_sub(art_width) / 2;

    // Build styled lines with ripple animation
    let styled_lines: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(row, line)| {
            let spans: Vec<Span> = line
                .chars()
                .enumerate()
                .map(|(col, ch)| {
                    if ch == ' ' {
                        Span::raw(" ")
                    } else {
                        let color = calculate_ripple_color(row, col, animation_frame);
                        Span::styled(ch.to_string(), Style::default().fg(color))
                    }
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    // Create paragraph with vertical centering via padding
    let mut padded_lines = Vec::new();

    // Add top padding
    for _ in 0..vertical_padding {
        padded_lines.push(Line::from(""));
    }

    // Add horizontal padding to each line
    for line in styled_lines {
        let padding = " ".repeat(horizontal_padding as usize);
        let mut spans = vec![Span::raw(padding)];
        spans.extend(line.spans);
        padded_lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(padded_lines);
    frame.render_widget(paragraph, area);
}

/// Calculates the color for a character based on ripple animation.
///
/// Creates a diagonal wave pattern that moves across the art,
/// with smooth color transitions between warm tones.
fn calculate_ripple_color(row: usize, col: usize, animation_frame: u32) -> Color {
    // Create diagonal wave pattern
    // The wave moves diagonally from top-left to bottom-right
    let wave_position = (row + col) as f32;

    // Animation speed: complete cycle every ~120 frames (2 seconds at 60fps)
    let time = animation_frame as f32 / 15.0;

    // Calculate wave intensity using sine for smooth transitions
    // Multiple waves with different frequencies for a mesh-like effect
    let wave1 = ((wave_position * 0.15 - time).sin() + 1.0) / 2.0;
    let wave2 = ((wave_position * 0.08 + time * 0.7).sin() + 1.0) / 2.0;

    // Combine waves for mesh effect
    let intensity = (wave1 * 0.6 + wave2 * 0.4).clamp(0.0, 1.0);

    // Only color characters above a threshold for subtle effect
    if intensity > 0.3 {
        // Map intensity to color palette index
        let palette_position = ((intensity - 0.3) / 0.7 * (WARM_COLORS.len() - 1) as f32) as usize;
        WARM_COLORS[palette_position.min(WARM_COLORS.len() - 1)]
    } else {
        BASE_COLOR
    }
}

