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

/// Soft warm color palette for the ripple animation using ANSI 256-color palette.
/// These muted, closely-spaced tones create a subtle gradient effect that flows
/// smoothly without harsh transitions.
const WARM_COLORS: [Color; 5] = [
    Color::Indexed(137), // Light khaki/tan
    Color::Indexed(173), // Soft salmon
    Color::Indexed(180), // Light peach
    Color::Indexed(144), // Light olive/sage
    Color::Indexed(137), // Back to khaki for smooth cycle
];

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
#[allow(clippy::cast_possible_truncation)] // ASCII art dimensions always fit in u16
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
/// Creates a gentle, flowing wave pattern that drifts across the art,
/// with very smooth color transitions between soft warm tones.
#[allow(clippy::cast_precision_loss)] // Precision loss acceptable for animation math
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Palette index always in range
#[allow(clippy::suboptimal_flops, clippy::manual_midpoint)] // Readable animation math preferred
fn calculate_ripple_color(row: usize, col: usize, animation_frame: u32) -> Color {
    // Create gentle diagonal wave pattern
    let wave_position = (row + col) as f32;

    // Slower animation: complete cycle every ~240 frames (4 seconds at 60fps)
    let time = animation_frame as f32 / 30.0;

    // Single smooth wave with very gentle frequency for a calm, flowing effect
    let wave = ((wave_position * 0.06 - time).sin() + 1.0) / 2.0;

    // Apply easing for even smoother transitions (ease-in-out curve)
    let eased = wave * wave * (3.0 - 2.0 * wave);

    // Map to palette with smooth interpolation
    // Scale to palette range (0 to len-1)
    let palette_pos = eased * (WARM_COLORS.len() - 1) as f32;
    let index = (palette_pos as usize).min(WARM_COLORS.len() - 1);

    WARM_COLORS[index]
}
