pub fn make_colored_icon(r: u8, g: u8, b: u8) -> tray_icon::Icon {
    const S: u32 = 32;
    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    let c = S as f32 / 2.0;
    let rad = S as f32 / 2.0 - 1.0;
    for y in 0..S {
        for x in 0..S {
            let dx = x as f32 - c;
            let dy = y as f32 - c;
            if dx * dx + dy * dy <= rad * rad {
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, S, S).expect("valid icon")
}

pub fn icon_for_status(status: Option<&str>) -> tray_icon::Icon {
    match status {
        Some("Recording...")  => make_colored_icon(239, 68,  68),  // red
        Some("Processing...") => make_colored_icon(251, 146, 60),  // orange
        Some("Distilling...") => make_colored_icon(168, 85,  247), // purple
        _                     => make_colored_icon(34,  197, 94),  // green (idle)
    }
}
