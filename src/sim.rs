//! A small drive-cycle simulation for the mocks: idle → accelerate (with gear
//! shifts) → cruise → decelerate → idle, repeating; coolant warms over minutes;
//! MAF, load and MAP follow RPM and throttle; fuel trims wander; voltage
//! jitters. Deterministic in `t` (seconds since the mock connected), so the same
//! moment always answers the same bytes. `OMATACH_MOCK_STATIC=1` turns it off.
/// Simulated vehicle state at time `t`.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub rpm: f64,
    pub speed_kmh: f64,
    pub throttle_pct: f64,
    pub load_pct: f64,
    pub coolant_c: f64,
    pub oil_c: f64,
    pub iat_c: f64,
    pub maf_gs: f64,
    pub map_kpa: f64,
    pub volts: f64,
    pub stft_pct: f64,
    pub ltft_pct: f64,
    pub fuel_pct: f64,
    pub runtime_s: f64,
}

const CYCLE: f64 = 60.0;
const IDLE1: f64 = 8.0;
const ACCEL: f64 = 14.0;
const CRUISE: f64 = 20.0;
const DECEL: f64 = 10.0;
const TOP_SPEED: f64 = 92.0;

fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub fn state(t: f64) -> State {
    let t = t.max(0.0);
    let p = t % CYCLE;
    // speed + throttle by phase
    let (speed, throttle) = if p < IDLE1 {
        (0.0, 4.0)
    } else if p < IDLE1 + ACCEL {
        let x = (p - IDLE1) / ACCEL;
        (TOP_SPEED * smooth(x), 62.0 - 30.0 * x)
    } else if p < IDLE1 + ACCEL + CRUISE {
        let x = p - IDLE1 - ACCEL;
        (TOP_SPEED + 3.0 * (x * 0.6).sin(), 18.0 + 3.0 * (x * 0.9).sin())
    } else if p < IDLE1 + ACCEL + CRUISE + DECEL {
        let x = (p - IDLE1 - ACCEL - CRUISE) / DECEL;
        (TOP_SPEED * (1.0 - smooth(x)), 1.0)
    } else {
        (0.0, 4.0)
    };
    // rpm: idle, else a per-gear sawtooth over 22 km/h bands + throttle bump
    let rpm = if speed < 1.0 {
        750.0 + 12.0 * (t * 2.3).sin()
    } else {
        let band = 22.0;
        let pos = (speed % band) / band;
        900.0 + speed * 10.0 + pos * 1400.0 + throttle * 8.0
    };
    let load = (throttle * 1.2 + 14.0).min(100.0);
    let maf = rpm * 0.0142 * (0.3 + 0.7 * load / 100.0);
    let map = 30.0 + load * 0.7;
    let coolant = 20.0 + 70.0 * (1.0 - (-t / 180.0).exp());
    let oil = coolant - 6.0 + 4.0 * (1.0 - (-t / 400.0).exp());
    let iat = 25.0 + 2.0 * (t / 40.0).sin();
    let volts = if speed < 1.0 { 13.9 } else { 14.15 } + 0.04 * (t * 3.1).sin();
    let stft = 2.5 * (t / 7.0).sin();
    let ltft = 1.6;
    let fuel = (62.0 - t / 600.0).max(5.0);
    State { rpm, speed_kmh: speed, throttle_pct: throttle, load_pct: load, coolant_c: coolant, oil_c: oil, iat_c: iat, maf_gs: maf, map_kpa: map, volts, stft_pct: stft, ltft_pct: ltft, fuel_pct: fuel, runtime_s: t }
}

fn u16be(v: f64) -> Vec<u8> {
    let n = v.round().clamp(0.0, 65535.0) as u16;
    vec![(n >> 8) as u8, (n & 0xFF) as u8]
}

fn u8c(v: f64) -> Vec<u8> {
    vec![v.round().clamp(0.0, 255.0) as u8]
}

/// J1979 data bytes for a mode-01 PID at time `t`, or `None` if not simulated.
pub fn bytes(pid: &str, t: f64) -> Option<Vec<u8>> {
    let s = state(t);
    Some(match pid.to_uppercase().as_str() {
        "04" => u8c(s.load_pct * 255.0 / 100.0),
        "05" => u8c(s.coolant_c + 40.0),
        "06" => u8c(s.stft_pct * 128.0 / 100.0 + 128.0),
        "07" => u8c(s.ltft_pct * 128.0 / 100.0 + 128.0),
        "0B" => u8c(s.map_kpa),
        "0C" => u16be(s.rpm * 4.0),
        "0D" => u8c(s.speed_kmh),
        "0F" => u8c(s.iat_c + 40.0),
        "10" => u16be(s.maf_gs * 100.0),
        "11" => u8c(s.throttle_pct * 255.0 / 100.0),
        "1F" => u16be(s.runtime_s),
        "2F" => u8c(s.fuel_pct * 255.0 / 100.0),
        "33" => u8c(101.0),
        "42" => u16be(s.volts * 1000.0),
        "45" => u8c(s.throttle_pct * 255.0 / 100.0),
        "46" => u8c(22.0 + 40.0),
        "5C" => u8c(s.oil_c + 40.0),
        _ => return None,
    })
}

/// Rewrite one captured single-frame mode-01 line's data bytes (`… <len> 41 <pid> <data…>`).
pub fn rewrite_line(line: &str, pid: &str, data: &[u8]) -> Option<String> {
    let toks: Vec<&str> = line.split_whitespace().collect();
    let i = toks.iter().position(|t| t.eq_ignore_ascii_case("41"))?;
    if !toks.get(i + 1)?.eq_ignore_ascii_case(pid) || i == 0 {
        return None;
    }
    let mut out: Vec<String> = toks[..i - 1].iter().map(|t| t.to_string()).collect();
    out.push(format!("{:02X}", 2 + data.len()));
    out.push("41".into());
    out.push(pid.to_uppercase());
    out.extend(data.iter().map(|b| format!("{b:02X}")));
    Some(out.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_cycle_has_idle_accel_and_cruise() {
        let idle = state(2.0);
        assert!((idle.rpm - 750.0).abs() < 20.0 && idle.speed_kmh == 0.0, "{idle:?}");
        let accel = state(IDLE1 + ACCEL * 0.6);
        assert!(accel.speed_kmh > 30.0 && accel.rpm > 1500.0 && accel.throttle_pct > 30.0, "{accel:?}");
        let cruise = state(IDLE1 + ACCEL + 5.0);
        assert!((cruise.speed_kmh - TOP_SPEED).abs() < 5.0, "{cruise:?}");
        let decel = state(IDLE1 + ACCEL + CRUISE + DECEL * 0.5);
        assert!(decel.speed_kmh < cruise.speed_kmh && decel.throttle_pct < 2.0, "{decel:?}");
        assert!(state(300.0).coolant_c > state(30.0).coolant_c, "coolant warms");
        assert!(state(0.0).coolant_c < 25.0 && state(1000.0).coolant_c > 85.0);
    }

    #[test]
    fn bytes_round_trip_through_the_catalog() {
        let mut c = crate::catalog::Catalog::from_vault();
        let s = state(20.0);
        let rpm = c.decode("010C.1", &bytes("0C", 20.0).unwrap()).unwrap().as_f64().unwrap();
        assert!((rpm - s.rpm).abs() < 0.3, "{rpm} vs {}", s.rpm);
        let ect = c.decode("0105.1", &bytes("05", 20.0).unwrap()).unwrap().as_f64().unwrap();
        assert!((ect - s.coolant_c).abs() < 1.0);
        let v = c.decode("0142.1", &bytes("42", 20.0).unwrap()).unwrap().as_f64().unwrap();
        assert!((v - s.volts).abs() < 0.01);
        assert!(bytes("ZZ", 0.0).is_none());
    }

    #[test]
    fn rewrite_keeps_headers_and_fixes_length() {
        assert_eq!(rewrite_line("7E8 04 41 0C 0B B8", "0C", &[0x1F, 0x40]).as_deref(), Some("7E8 04 41 0C 1F 40"));
        assert_eq!(rewrite_line("18 DA F1 10 03 41 05 83", "05", &[0x90]).as_deref(), Some("18 DA F1 10 03 41 05 90"));
        assert_eq!(rewrite_line("7E8 03 41 05 83", "0C", &[0, 0]), None);
    }
}
