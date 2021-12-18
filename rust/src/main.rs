use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone};
use clap::Parser;
use fitparser;
use fitparser::profile::MesgNum;
use geo;
use geo::prelude::Contains;
use geo::vincenty_distance::VincentyDistance;
use serde_json;


trait GeoPoint {
    fn latitude(&self) -> f64;
    fn longitude(&self) -> f64;
    fn as_lonlat_list(&self) -> serde_json::Value {
        serde_json::json!([
            self.longitude(),
            self.latitude(),
        ])
    }
}


#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    pub coordinates_deg: geo::Point<f64>,
    pub elevation_m: Option<f64>,
    pub unix_timestamp: Option<f64>,
    pub heart_rate_bpm: Option<u64>,
    pub speed_km_per_h: Option<f64>,
    pub cadence_rpm: Option<u64>,
    pub temperature_degc: Option<i64>,
    pub timestamp: Option<DateTime<Local>>,
}
impl Point {
    pub fn new(
        coordinates_deg: geo::Point<f64>,
        elevation_m: Option<f64>,
        unix_timestamp: Option<f64>,
        heart_rate_bpm: Option<u64>,
        speed_km_per_h: Option<f64>,
        cadence_rpm: Option<u64>,
        temperature_degc: Option<i64>,
        timestamp: Option<DateTime<Local>>,
    ) -> Self {
        Self {
            coordinates_deg,
            elevation_m,
            unix_timestamp,
            heart_rate_bpm,
            speed_km_per_h,
            cadence_rpm,
            temperature_degc,
            timestamp,
        }
    }
}
impl GeoPoint for Point {
    #[inline]
    fn latitude(&self) -> f64 {
        self.coordinates_deg.y()
    }

    #[inline]
    fn longitude(&self) -> f64 {
        self.coordinates_deg.x()
    }
}

fn avg<T, A, J>(v1: Option<T>, v2: Option<T>, mut average: A, mut jsonify: J) -> Option<serde_json::Value>
    where
        A : FnMut(T, T) -> T,
        J : FnMut(T) -> serde_json::Value,
{
    let avg = match (v1, v2) {
        (None, None) => None,
        (Some(s1), None) => Some(s1),
        (None, Some(s2)) => Some(s2),
        (Some(s1), Some(s2)) => Some(average(s1, s2)),
    };
    avg.map(|v| jsonify(v))
}

fn f64_avg(f1: Option<f64>, f2: Option<f64>) -> Option<serde_json::Value> {
    avg(
        f1, f2,
        |a, b| (a + b)/2.0,
        |v| serde_json::Value::Number(serde_json::Number::from_f64(v).unwrap()),
    )
}

fn i64_avg(i1: Option<i64>, i2: Option<i64>) -> Option<serde_json::Value> {
    avg(
        i1, i2,
        |a, b| (a + b)/2,
        |v| serde_json::Value::Number(serde_json::Number::from(v)),
    )
}

fn u64_avg(i1: Option<u64>, i2: Option<u64>) -> Option<serde_json::Value> {
    avg(
        i1, i2,
        |a, b| (a + b)/2,
        |v| serde_json::Value::Number(serde_json::Number::from(v)),
    )
}

fn time_avg(t1: Option<DateTime<Local>>, t2: Option<DateTime<Local>>) -> Option<serde_json::Value> {
    avg(
        t1, t2,
        |a, b| Local.timestamp((a.timestamp() + b.timestamp()) / 2, 0),
        |v| serde_json::Value::String(v.format("%Y-%m-%d %H:%M:%S").to_string()),
    )
}

fn lines_to_track(lines: &Vec<Vec<Point>>) -> serde_json::Value {
    let mut features = Vec::new();
    for line in lines {
        let coordinates: Vec<serde_json::Value> = line
            .iter()
            .map(|p| serde_json::json!([p.longitude(), p.latitude()]))
            .collect();
        let json_line = serde_json::json!({
            "type": "Feature",
            "properties": {},
            "geometry": {
                "type": "LineString",
                "coordinates": coordinates,
            },
        });
        features.push(json_line);
    }
    serde_json::json!({
        "type": "FeatureCollection",
        "features": features,
    })
}

fn lines_to_points(lines: &Vec<Vec<Point>>) -> serde_json::Value {
    let mut features = Vec::new();
    let mut running_dist_m = 0.0;
    for line in lines {
        for i in 0..line.len()-1 {
            let point1 = &line[i];
            let point2 = &line[i+1];
            let dist_m = point1.coordinates_deg.vincenty_distance(&point2.coordinates_deg)
                .expect("distance calculation failed to converge");
            running_dist_m += dist_m;

            let mut properties = serde_json::Map::new();
            properties.insert("running_distance".to_owned(), running_dist_m.into());
            if let Some(spd) = f64_avg(point1.speed_km_per_h, point2.speed_km_per_h) {
                properties.insert("speed".to_owned(), spd);
            }
            if let Some(ele) = f64_avg(point1.elevation_m, point2.elevation_m) {
                properties.insert("elevation".to_owned(), ele);
            }
            if let Some(hr) = u64_avg(point1.heart_rate_bpm, point2.heart_rate_bpm) {
                properties.insert("heart_rate".to_owned(), hr);
            }
            if let Some(cad) = u64_avg(point1.cadence_rpm, point2.cadence_rpm) {
                properties.insert("cadence".to_owned(), cad);
            }
            if let Some(temp) = i64_avg(point1.temperature_degc, point2.temperature_degc) {
                properties.insert("temperature".to_owned(), temp);
            }
            if let Some(time) = time_avg(point1.timestamp, point2.timestamp) {
                properties.insert("timestamp".to_owned(), time);
            }

            let feature = serde_json::json!({
                "type": "Feature",
                "properties": properties,
                "geometry": {
                    "type": "LineString",
                    "coordinates": [
                        point1.as_lonlat_list(),
                        point2.as_lonlat_list(),
                    ],
                },
            });
            features.push(feature);
        }
    }

    serde_json::json!({
        "type": "FeatureCollection",
        "features": features,
    })
}

#[inline]
fn semicircles_to_degrees(sc: f64) -> f64 {
    sc * 180.0 / 2.0_f64.powi(31)
}

fn semicircle_value_to_degrees(sc_val: &fitparser::Value) -> f64 {
    if let fitparser::Value::SInt32(sc) = sc_val {
        semicircles_to_degrees((*sc).into())
    } else {
        panic!("unexpected value {:?}", sc_val);
    }
}


fn coord_extrema<F>(lines: &Vec<Vec<Point>>, mut coord: F) -> Option<(f64, f64)>
    where
        F: FnMut(&Point) -> Option<f64>,
{
    let min = lines.iter()
        .flat_map(|l| l.iter())
        .filter_map(|p| coord(p))
        .reduce(f64::min);
    let max = lines.iter()
        .flat_map(|l| l.iter())
        .filter_map(|p| coord(p))
        .reduce(f64::max);

    if let (Some(mn), Some(mx)) = (min, max) {
        Some((mn, mx))
    } else {
        None
    }
}


fn load_censor_polygon(path: &Path) -> geo::Polygon<f64> {
    let buf = {
        let mut file = File::open(path)
            .expect("failed to open polygon file");
        let mut buf = String::new();
        file.read_to_string(&mut buf)
            .expect("failed to read polygon file");
        buf
    };

    let mut points = Vec::new();
    for (i, mut line) in buf.split('\n').enumerate() {
        if let Some(hash_index) = line.find('#') {
            line = &line[..hash_index];
        }
        line = line.trim();

        if line.len() == 0 {
            // empty line (or comment only)
            continue;
        }

        let coord_pieces: Vec<&str> = line.split_whitespace().collect();
        if coord_pieces.len() != 2 {
            panic!("line {}: {} coordinate pieces; expected 2", i+1, coord_pieces.len());
        }

        let lat: f64 = match coord_pieces[0].replace("_", "").parse() {
            Ok(l) => l,
            Err(e) => panic!("line {}: failed to parse {:?} as a latitude: {}", i + 1, coord_pieces[0], e),
        };
        let lon: f64 = match coord_pieces[1].replace("_", "").parse() {
            Ok(l) => l,
            Err(e) => panic!("line {}: failed to parse {:?} as a longitude: {}", i + 1, coord_pieces[1], e),
        };
        points.push(geo::Coordinate { x: lon, y: lat });
    }

    geo::Polygon::new(geo::LineString::from(points), vec![])
}


#[derive(Clone, Debug, Eq, Hash, Parser, PartialEq)]
struct Opts {
    #[clap(short, long)] pub events: bool,
    #[clap(short, long)] pub no_records: bool,
    #[clap(short, long = "censor-polygon", multiple_occurrences = true, multiple_values = false)] pub censor_polygons: Vec<PathBuf>,
    #[clap(required = true)] pub filenames: Vec<PathBuf>,
}


fn main() {
    let opts = Opts::parse();

    for filename in &opts.filenames {
        let mut file = File::open(filename)
            .expect("failed to open file");

        let mut lines = Vec::new();
        let mut line = Vec::new();

        let censor_polygons: Vec<geo::Polygon<f64>> = opts.censor_polygons.iter()
            .map(|cp| load_censor_polygon(cp))
            .collect();

        for record in fitparser::from_reader(&mut file).expect("failed to read file") {
            if opts.events && (!opts.no_records || record.kind() != MesgNum::Record) {
                eprintln!("{:?}", record.kind());
                for field in record.fields() {
                    eprintln!("  {}[{}] = {:?} {}", field.name(), field.number(), field.value(), field.units());
                }
            }

            if record.kind() == MesgNum::Event {
                let event_category_opt = record.fields().iter()
                    .filter(|f| f.number() == 0)
                    .map(|f| f.value())
                    .nth(0);
                let event_type_opt = record.fields().iter()
                    .filter(|f| f.number() == 1)
                    .map(|f| f.value())
                    .nth(0);

                if let Some(fitparser::Value::String(ec)) = event_category_opt {
                    if ec == "timer" {
                        if let Some(fitparser::Value::String(et)) = event_type_opt {
                            if et == "stop_all" {
                                // timer stopped; show this as a discontinuity in the line
                                if line.len() > 0 {
                                    lines.push(line);
                                }
                                line = Vec::new();
                            }
                        }
                    }
                }
            }

            if record.kind() != MesgNum::Record {
                continue;
            }

            let lat_semicirc_opt = record.fields().iter()
                .filter(|df| df.name() == "position_lat")
                .nth(0);
            let lon_semicirc_opt = record.fields().iter()
                .filter(|df| df.name() == "position_long")
                .nth(0);

            if lat_semicirc_opt.is_none() || lon_semicirc_opt.is_none() {
                // position recording paused (probably went indoors)
                // store the current line and try the next point
                if line.len() > 0 {
                    lines.push(line);
                    line = Vec::new();
                }
                continue;
            }

            let lat_semicirc = lat_semicirc_opt.unwrap();
            let lon_semicirc = lon_semicirc_opt.unwrap();

            let lat_deg = semicircle_value_to_degrees(lat_semicirc.value());
            let lon_deg = semicircle_value_to_degrees(lon_semicirc.value());
            let coords_deg = geo::Coordinate { x: lon_deg, y: lat_deg };

            if censor_polygons.iter().any(|cp| cp.contains(&coords_deg)) {
                // skip this point; it is censored
                continue;
            }

            let mut final_timestamp = None;
            let timestamp_field_opt = record.fields().iter()
                .filter(|df| df.name() == "timestamp")
                .nth(0);
            if let Some(tsfield) = timestamp_field_opt {
                if let fitparser::Value::Timestamp(ts) = tsfield.value() {
                    let ts_ms = ts.timestamp_millis();
                    let ts_f64 = (ts_ms as f64) / 1000.0;
                    final_timestamp = Some(ts_f64);
                }
            }

            let mut final_heart_rate = None;
            let hr_field_opt = record.fields().iter()
                .filter(|df| df.name() == "heart_rate")
                .nth(0);
            if let Some(hr_field) = hr_field_opt {
                if let fitparser::Value::UInt8(hr) = hr_field.value() {
                    final_heart_rate = Some((*hr) as u64);
                }
            }

            let mut final_altitude = None;
            let alt_field_opt = record.fields().iter()
                .filter(|df| df.name() == "enhanced_altitude")
                .nth(0);
            if let Some(alt_field) = alt_field_opt {
                if let fitparser::Value::Float64(alt) = alt_field.value() {
                    final_altitude = Some(*alt);
                }
            }

            let mut final_speed_km_per_h = None;
            let speed_field_opt = record.fields().iter()
                .filter(|df| df.name() == "enhanced_speed")
                .nth(0);
            if let Some(speed_field) = speed_field_opt {
                if let fitparser::Value::Float64(speed_mpers) = speed_field.value() {
                    let speed_km_per_h = (*speed_mpers) * 3.6;
                    final_speed_km_per_h = Some(speed_km_per_h);
                }
            }

            let mut final_cadence = None;
            let cadence_field_opt = record.fields().iter()
                .filter(|df| df.name() == "cadence")
                .nth(0);
            if let Some(cadence_field) = cadence_field_opt {
                if let fitparser::Value::UInt8(cad) = cadence_field.value() {
                    final_cadence = Some((*cad) as u64);
                }
            }

            let mut final_temperature = None;
            let temperature_field_opt = record.fields().iter()
                .filter(|df| df.name() == "temperature")
                .nth(0);
            if let Some(temperature_field) = temperature_field_opt {
                if let fitparser::Value::SInt8(temp) = temperature_field.value() {
                    final_temperature = Some((*temp) as i64);
                }
            }

            let mut final_time = None;
            let time_field_opt = record.fields().iter()
                .filter(|df| df.name() == "timestamp")
                .nth(0);
            if let Some(time_field) = time_field_opt {
                if let fitparser::Value::Timestamp(ts) = time_field.value() {
                    final_time = Some(*ts);
                }
            }

            let point = Point::new(
                geo::Point::from((lon_deg, lat_deg)),
                final_altitude,
                final_timestamp,
                final_heart_rate,
                final_speed_km_per_h,
                final_cadence,
                final_temperature,
                final_time,
            );
            //println!("{:?}", point);
            line.push(point);
        }

        // store final line
        if line.len() > 0 {
            lines.push(line);
        }

        // convert to GeoJSON
        let track = lines_to_track(&lines);
        let points = lines_to_points(&lines);

        // find coordinate extrema (assume we never go over the 180° meridian)
        let (min_lat, max_lat) = coord_extrema(&lines, |p| Some(p.latitude())).unwrap();
        let (min_lon, max_lon) = coord_extrema(&lines, |p| Some(p.longitude())).unwrap();
        let avg_lat = (min_lat + max_lat)/2.0;
        let avg_lon = (min_lon + max_lon)/2.0;
        let (min_ele, max_ele) = coord_extrema(&lines, |p| p.elevation_m).unwrap();
        let (min_hr, max_hr) = coord_extrema(&lines, |p| p.heart_rate_bpm.map(|hr| hr as f64))
            .unwrap_or((80.0, 160.0));
        let (min_speed, max_speed) = coord_extrema(&lines, |p| p.speed_km_per_h)
            .unwrap_or((0.0, 10.0));
        let (min_cad, max_cad) = coord_extrema(&lines, |p| p.cadence_rpm.map(|hr| hr as f64))
            .unwrap_or((0.0, 120.0));
        let (min_temp, max_temp) = coord_extrema(&lines, |p| p.temperature_degc.map(|hr| hr as f64))
            .unwrap_or((-10.0, 45.0));

        let final_json = serde_json::json!({
            "center": [avg_lat, avg_lon],
            "zoom": 12, // FIXME: estimate this
            "track": track,
            "points": points,
            "elevation_range": [min_ele, max_ele],
            "heart_rate_range": [min_hr, max_hr],
            "speed_range": [min_speed, max_speed],
            "cadence_range": [min_cad, max_cad],
            "temperature_range": [min_temp, max_temp],
        });

        println!("{}", serde_json::to_string_pretty(&final_json).unwrap());
    }
}
