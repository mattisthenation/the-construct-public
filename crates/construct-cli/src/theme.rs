use ratatui::style::{Color, Style};

/// Earthy blues & browns palette for The Construct.
pub struct Theme;

impl Theme {
    pub const DEEP_BLUE: Color = Color::Rgb(38, 70, 83); // slate teal-blue
                                                         // Part of the named palette; not referenced in non-test builds yet.
    #[allow(dead_code)]
    pub const DUSK_BLUE: Color = Color::Rgb(69, 105, 124); // muted steel blue
    pub const CLAY: Color = Color::Rgb(122, 85, 58); // warm brown
    pub const SAND: Color = Color::Rgb(196, 164, 132); // tan
    pub const PARCHMENT: Color = Color::Rgb(231, 217, 196); // light foreground

    pub fn header() -> Style {
        Style::default().fg(Self::SAND).bg(Self::DEEP_BLUE)
    }
    pub fn accent() -> Style {
        Style::default().fg(Self::CLAY)
    }
    pub fn body() -> Style {
        Style::default().fg(Self::PARCHMENT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_is_earthy() {
        assert_eq!(Theme::DEEP_BLUE, Color::Rgb(38, 70, 83));
        assert_eq!(Theme::CLAY, Color::Rgb(122, 85, 58));
        // header uses sand on deep blue
        assert_eq!(
            Theme::header(),
            Style::default().fg(Theme::SAND).bg(Theme::DEEP_BLUE)
        );
    }
}
