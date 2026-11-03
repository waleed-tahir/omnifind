//! Custom UI styling for OmniFind matching modern Windows 11 Light Dashboard aesthetics.

use iced::widget::{button, container};
use iced::{Background, Border, Color, Shadow, Theme, Vector};

pub const COLOR_WINDOW_BG: Color = Color::from_rgb(0.96, 0.97, 0.99); // #f4f6fb soft ambient
pub const COLOR_SIDEBAR_BG: Color = Color::from_rgb(0.93, 0.94, 0.97); // #edf0f7
pub const COLOR_CARD_BG: Color = Color::from_rgb(1.0, 1.0, 1.0); // #ffffff
pub const COLOR_CARD_BORDER: Color = Color::from_rgb(0.88, 0.90, 0.94); // #e1e6f0
pub const COLOR_TEXT_MAIN: Color = Color::from_rgb(0.11, 0.14, 0.19); // #1c2430 deep charcoal
pub const COLOR_TEXT_MUTED: Color = Color::from_rgb(0.45, 0.50, 0.60); // #738099
pub const COLOR_PRIMARY: Color = Color::from_rgb(0.18, 0.45, 0.95); // #2e73f2 vibrant blue
pub const COLOR_PRIMARY_HOVER: Color = Color::from_rgb(0.14, 0.38, 0.85);
pub const COLOR_CHIP_BG: Color = Color::from_rgb(0.92, 0.94, 0.98); // #ebf0fa
pub const COLOR_CHIP_TEXT: Color = Color::from_rgb(0.18, 0.32, 0.60);
pub const COLOR_NAV_ACTIVE_BG: Color = Color::from_rgb(0.86, 0.89, 0.96); // #dce3f5

pub struct CardStyle;
impl container::StyleSheet for CardStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: Some(COLOR_TEXT_MAIN),
            background: Some(Background::Color(COLOR_CARD_BG)),
            border: Border {
                color: COLOR_CARD_BORDER,
                width: 1.0,
                radius: 12.0.into(),
            },
            shadow: Shadow::default(),
        }
    }
}

pub struct SidebarStyle;
impl container::StyleSheet for SidebarStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: Some(COLOR_TEXT_MAIN),
            background: Some(Background::Color(COLOR_SIDEBAR_BG)),
            border: Border {
                color: COLOR_CARD_BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            shadow: Shadow::default(),
        }
    }
}

pub struct WindowBgStyle;
impl container::StyleSheet for WindowBgStyle {
    type Style = Theme;
    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: Some(COLOR_TEXT_MAIN),
            background: Some(Background::Color(COLOR_WINDOW_BG)),
            border: Border::default(),
            shadow: Shadow::default(),
        }
    }
}

pub struct PrimaryPillBtn;
impl button::StyleSheet for PrimaryPillBtn {
    type Style = Theme;
    fn active(&self, _style: &Self::Style) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(COLOR_PRIMARY)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 20.0.into(),
            },
            text_color: Color::WHITE,
            shadow_offset: Vector::default(),
            shadow: Shadow::default(),
        }
    }
    fn hovered(&self, _style: &Self::Style) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(COLOR_PRIMARY_HOVER)),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: 20.0.into(),
            },
            text_color: Color::WHITE,
            shadow_offset: Vector::default(),
            shadow: Shadow::default(),
        }
    }
}

pub struct SecondaryPillBtn;
impl button::StyleSheet for SecondaryPillBtn {
    type Style = Theme;
    fn active(&self, _style: &Self::Style) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(COLOR_CARD_BG)),
            border: Border {
                color: COLOR_CARD_BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: COLOR_TEXT_MAIN,
            shadow_offset: Vector::default(),
            shadow: Shadow::default(),
        }
    }
    fn hovered(&self, _style: &Self::Style) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.96, 0.97, 0.99))),
            border: Border {
                color: Color::from_rgb(0.75, 0.80, 0.88),
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: COLOR_PRIMARY,
            shadow_offset: Vector::default(),
            shadow: Shadow::default(),
        }
    }
}

pub struct QuickChipBtn;
impl button::StyleSheet for QuickChipBtn {
    type Style = Theme;
    fn active(&self, _style: &Self::Style) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(COLOR_CHIP_BG)),
            border: Border {
                color: Color::from_rgb(0.85, 0.88, 0.94),
                width: 1.0,
                radius: 14.0.into(),
            },
            text_color: COLOR_CHIP_TEXT,
            shadow_offset: Vector::default(),
            shadow: Shadow::default(),
        }
    }
    fn hovered(&self, _style: &Self::Style) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.85, 0.89, 0.98))),
            border: Border {
                color: COLOR_PRIMARY,
                width: 1.0,
                radius: 14.0.into(),
            },
            text_color: COLOR_PRIMARY,
            shadow_offset: Vector::default(),
            shadow: Shadow::default(),
        }
    }
}

pub struct NavItemBtn {
    pub is_active: bool,
}
impl button::StyleSheet for NavItemBtn {
    type Style = Theme;
    fn active(&self, _style: &Self::Style) -> button::Appearance {
        if self.is_active {
            button::Appearance {
                background: Some(Background::Color(COLOR_NAV_ACTIVE_BG)),
                border: Border {
                    color: Color::from_rgb(0.78, 0.83, 0.92),
                    width: 1.0,
                    radius: 8.0.into(),
                },
                text_color: COLOR_TEXT_MAIN,
                shadow_offset: Vector::default(),
                shadow: Shadow::default(),
            }
        } else {
            button::Appearance {
                background: None,
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: 8.0.into(),
                },
                text_color: COLOR_TEXT_MUTED,
                shadow_offset: Vector::default(),
                shadow: Shadow::default(),
            }
        }
    }
    fn hovered(&self, _style: &Self::Style) -> button::Appearance {
        button::Appearance {
            background: Some(Background::Color(Color::from_rgb(0.89, 0.92, 0.97))),
            border: Border {
                color: Color::from_rgb(0.80, 0.85, 0.92),
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: COLOR_TEXT_MAIN,
            shadow_offset: Vector::default(),
            shadow: Shadow::default(),
        }
    }
}
