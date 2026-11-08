use crate::traffic_icon::DirectionIcon;
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub(super) struct UiAssets;

const STATIC_ASSETS: &[(&str, &[u8])] = &[
    (
        "icons/application.svg",
        include_bytes!("../../assets/application.svg"),
    ),
    (
        "images/traffic.png",
        include_bytes!("../../assets/traffic.png"),
    ),
    (
        "images/tray-dark.png",
        include_bytes!("../../assets/tray-dark.png"),
    ),
    (
        "images/tray-light.png",
        include_bytes!("../../assets/tray-light.png"),
    ),
    ("icons/check.svg", include_bytes!("../../assets/check.svg")),
    (
        "icons/arrow-left.svg",
        include_bytes!("../../assets/arrow-left.svg"),
    ),
    (
        "icons/arrow-right.svg",
        include_bytes!("../../assets/arrow-right.svg"),
    ),
    (
        "icons/arrow-up.svg",
        include_bytes!("../../assets/arrow-up.svg"),
    ),
    (
        "icons/history-trend.svg",
        include_bytes!("../../assets/history-trend.svg"),
    ),
    (
        "icons/arrow-down.svg",
        include_bytes!("../../assets/arrow-down.svg"),
    ),
    ("icons/edit.svg", include_bytes!("../../assets/edit.svg")),
    (
        "icons/calendar.svg",
        include_bytes!("../../assets/calendar.svg"),
    ),
    ("icons/date.svg", include_bytes!("../../assets/date.svg")),
    ("icons/clock.svg", include_bytes!("../../assets/clock.svg")),
    ("icons/undo.svg", include_bytes!("../../assets/undo.svg")),
    (
        "icons/locate.svg",
        include_bytes!("../../assets/locate.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../../assets/chevron-down.svg"),
    ),
    (
        "icons/chevron-up.svg",
        include_bytes!("../../assets/chevron-up.svg"),
    ),
    (
        "icons/sidebar-collapse.svg",
        include_bytes!("../../assets/sidebar-collapse.svg"),
    ),
    (
        "icons/sidebar-expand.svg",
        include_bytes!("../../assets/sidebar-expand.svg"),
    ),
    (
        "icons/sidebar-apps.svg",
        include_bytes!("../../assets/sidebar-apps.svg"),
    ),
    ("icons/globe.svg", include_bytes!("../../assets/globe.svg")),
    (
        "icons/carousel-play.svg",
        include_bytes!("../../assets/carousel-play.svg"),
    ),
    (
        "icons/carousel-pause.svg",
        include_bytes!("../../assets/carousel-pause.svg"),
    ),
    (
        "icons/outbound-distribution.svg",
        include_bytes!("../../assets/outbound-distribution.svg"),
    ),
    (
        "icons/sidebar-connections.svg",
        include_bytes!("../../assets/sidebar-connections.svg"),
    ),
    (
        "icons/display-taskbar.svg",
        include_bytes!("../../assets/display-taskbar.svg"),
    ),
    (
        "icons/display-floating.svg",
        include_bytes!("../../assets/display-floating.svg"),
    ),
    (
        "icons/window-minimize.svg",
        include_bytes!("../../assets/window-minimize.svg"),
    ),
    (
        "icons/window-maximize.svg",
        include_bytes!("../../assets/window-maximize.svg"),
    ),
    (
        "icons/window-restore.svg",
        include_bytes!("../../assets/window-restore.svg"),
    ),
    (
        "icons/window-close.svg",
        include_bytes!("../../assets/window-close.svg"),
    ),
    (
        "icons/position-left.svg",
        include_bytes!("../../assets/position-left.svg"),
    ),
    (
        "icons/position-right.svg",
        include_bytes!("../../assets/position-right.svg"),
    ),
    (
        "icons/position-info.svg",
        include_bytes!("../../assets/position-info.svg"),
    ),
];
const DIRECTION_ICONS: &[(&str, DirectionIcon)] = &[
    ("icons/traffic-upload.svg", DirectionIcon::Upload),
    ("icons/traffic-download.svg", DirectionIcon::Download),
    ("icons/traffic-total.svg", DirectionIcon::TotalSpeed),
];

pub(super) fn country_flag_data(code: &str) -> Option<&'static [u8]> {
    // flag-icons 7.5.0, rasterized at 64×48. Each record is an ASCII code, a
    // little-endian u16 byte count, and a PNG. See assets/licenses/flag-icons.txt.
    let mut flags = include_bytes!("../../assets/country-flags.bin").as_slice();
    while let Some(header) = flags.get(..4) {
        let length = u16::from_le_bytes([header[2], header[3]]) as usize;
        let png = flags.get(4..4 + length)?;
        if code.as_bytes().eq_ignore_ascii_case(&header[..2]) {
            return Some(png);
        }
        flags = &flags[4 + length..];
    }
    None
}

impl AssetSource for UiAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(code) = path
            .strip_prefix("flags/")
            .and_then(|name| name.strip_suffix(".png"))
        {
            return Ok(country_flag_data(code).map(Cow::Borrowed));
        }
        if let Some((_, icon)) = DIRECTION_ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Owned(icon.svg())));
        }
        Ok(STATIC_ASSETS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(STATIC_ASSETS
            .iter()
            .map(|(name, _)| *name)
            .chain(DIRECTION_ICONS.iter().map(|(name, _)| *name))
            .filter(|name| name.starts_with(path))
            .map(Into::into)
            .collect())
    }
}
