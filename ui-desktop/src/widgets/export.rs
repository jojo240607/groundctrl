//! 轨迹导出：将 GPS 轨迹与航点导出为 CSV / KML 文本。

use groundctrl_core::vehicle::mission::Waypoint;

/// 将 GPS 轨迹导出为 CSV（seq,latitude,longitude）
pub fn export_track_csv(trail: &[(f64, f64)]) -> String {
    let mut s = String::from("seq,latitude,longitude\n");
    for (i, (la, lo)) in trail.iter().enumerate() {
        s.push_str(&format!("{i},{:.7},{:.7}\n", la, lo));
    }
    s
}

/// 将 GPS 轨迹与航点导出为 KML（LineString + Point Placemarks）
pub fn export_track_kml(trail: &[(f64, f64)], mission: &[Waypoint]) -> String {
    let mut coords: String = String::new();
    for (la, lo) in trail {
        coords.push_str(&format!("{:.7},{:.7},0\n", lo, la));
    }
    let mut wps: String = String::new();
    for (i, wp) in mission.iter().enumerate() {
        wps.push_str(&format!(
            "    <Placemark>\n      <name>WP#{i}</name>\n      <Point><coordinates>{:.7},{:.7},{}</coordinates></Point>\n    </Placemark>\n",
            wp.lon, wp.lat, wp.alt
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2">
  <Document>
    <name>GroundControl Track</name>
    <Placemark>
      <name>Track</name>
      <LineString>
        <coordinates>
{coords}        </coordinates>
      </LineString>
    </Placemark>
{wps}  </Document>
</kml>
"#
    )
}
