//! Maths: a safe calculator (no eval), conversions, finance and statistics.

use serde_json::json;

use crate::{Action, local, n, n_or, num, numbers, s, s_or};

const C: &str = "Math";

/// Recursive-descent calculator: + - * / % ^, parentheses, unary minus, and
/// sqrt, abs, ln, log, sin, cos, tan, round, floor, ceil, min, max, pi, e.
pub fn calculate(expr: &str) -> Result<f64, String> {
    struct P<'a> {
        s: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn eat(&mut self, c: u8) -> bool {
            self.ws();
            if self.s.get(self.i) == Some(&c) {
                self.i += 1;
                true
            } else {
                false
            }
        }
        fn expr(&mut self) -> Result<f64, String> {
            let mut v = self.term()?;
            loop {
                if self.eat(b'+') {
                    v += self.term()?;
                } else if self.eat(b'-') {
                    v -= self.term()?;
                } else {
                    return Ok(v);
                }
            }
        }
        fn term(&mut self) -> Result<f64, String> {
            let mut v = self.power()?;
            loop {
                if self.eat(b'*') {
                    v *= self.power()?;
                } else if self.eat(b'/') {
                    let d = self.power()?;
                    if d == 0.0 {
                        return Err("division by zero".into());
                    }
                    v /= d;
                } else if self.eat(b'%') {
                    v %= self.power()?;
                } else {
                    return Ok(v);
                }
            }
        }
        fn power(&mut self) -> Result<f64, String> {
            let base = self.unary()?;
            if self.eat(b'^') { Ok(base.powf(self.power()?)) } else { Ok(base) }
        }
        fn unary(&mut self) -> Result<f64, String> {
            if self.eat(b'-') {
                return Ok(-self.unary()?);
            }
            if self.eat(b'+') {
                return self.unary();
            }
            self.atom()
        }
        fn atom(&mut self) -> Result<f64, String> {
            self.ws();
            if self.eat(b'(') {
                let v = self.expr()?;
                if !self.eat(b')') {
                    return Err("missing )".into());
                }
                return Ok(v);
            }
            let start = self.i;
            if self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.' || *c == b'_') {
                    self.i += 1;
                }
                if self.s.get(self.i).is_some_and(|c| *c == b'e' || *c == b'E') {
                    self.i += 1;
                    if self.s.get(self.i).is_some_and(|c| *c == b'-' || *c == b'+') {
                        self.i += 1;
                    }
                    while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                }
                let t = std::str::from_utf8(&self.s[start..self.i]).unwrap().replace('_', "");
                return t.parse().map_err(|_| format!("bad number {t}"));
            }
            while self.s.get(self.i).is_some_and(u8::is_ascii_alphabetic) {
                self.i += 1;
            }
            let name = std::str::from_utf8(&self.s[start..self.i]).unwrap().to_ascii_lowercase();
            match name.as_str() {
                "pi" => return Ok(std::f64::consts::PI),
                "e" => return Ok(std::f64::consts::E),
                "" => return Err(format!("unexpected character at {}", self.i + 1)),
                _ => {}
            }
            if !self.eat(b'(') {
                return Err(format!("unknown name {name}"));
            }
            let mut args = vec![self.expr()?];
            while self.eat(b',') {
                args.push(self.expr()?);
            }
            if !self.eat(b')') {
                return Err("missing )".into());
            }
            let a = args[0];
            Ok(match name.as_str() {
                "sqrt" => a.sqrt(),
                "abs" => a.abs(),
                "ln" => a.ln(),
                "log" => a.log10(),
                "exp" => a.exp(),
                "sin" => a.sin(),
                "cos" => a.cos(),
                "tan" => a.tan(),
                "round" => {
                    let d = args.get(1).copied().unwrap_or(0.0);
                    let f = 10f64.powf(d);
                    (a * f).round() / f
                }
                "floor" => a.floor(),
                "ceil" => a.ceil(),
                "min" => args.iter().copied().fold(f64::INFINITY, f64::min),
                "max" => args.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                other => return Err(format!("unknown function {other}")),
            })
        }
    }
    let mut p = P { s: expr.as_bytes(), i: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("unexpected text at position {}", p.i + 1));
    }
    Ok(v)
}

/// Factor to a base unit per dimension; temperatures are handled separately.
fn unit(u: &str) -> Option<(&'static str, f64)> {
    let u = u.trim().to_lowercase();
    Some(match u.as_str() {
        "mm" | "millimeter" | "millimeters" => ("length", 0.001),
        "cm" | "centimeter" | "centimeters" => ("length", 0.01),
        "m" | "meter" | "meters" | "metre" | "metres" => ("length", 1.0),
        "km" | "kilometer" | "kilometers" => ("length", 1000.0),
        "in" | "inch" | "inches" => ("length", 0.0254),
        "ft" | "foot" | "feet" => ("length", 0.3048),
        "yd" | "yard" | "yards" => ("length", 0.9144),
        "mi" | "mile" | "miles" => ("length", 1609.344),
        "nmi" | "nautical mile" | "nautical miles" => ("length", 1852.0),
        "mg" | "milligram" | "milligrams" => ("mass", 0.000001),
        "g" | "gram" | "grams" => ("mass", 0.001),
        "kg" | "kilogram" | "kilograms" => ("mass", 1.0),
        "t" | "tonne" | "tonnes" => ("mass", 1000.0),
        "oz" | "ounce" | "ounces" => ("mass", 0.028349523125),
        "lb" | "lbs" | "pound" | "pounds" => ("mass", 0.45359237),
        "st" | "stone" => ("mass", 6.35029318),
        "ml" | "milliliter" | "milliliters" => ("volume", 0.001),
        "l" | "liter" | "liters" | "litre" | "litres" => ("volume", 1.0),
        "tsp" | "teaspoon" | "teaspoons" => ("volume", 0.00492892159375),
        "tbsp" | "tablespoon" | "tablespoons" => ("volume", 0.01478676478125),
        "cup" | "cups" => ("volume", 0.2365882365),
        "floz" | "fl oz" | "fluid ounce" | "fluid ounces" => ("volume", 0.0295735295625),
        "pt" | "pint" | "pints" => ("volume", 0.473176473),
        "qt" | "quart" | "quarts" => ("volume", 0.946352946),
        "gal" | "gallon" | "gallons" => ("volume", 3.785411784),
        "m/s" => ("speed", 1.0),
        "km/h" | "kph" => ("speed", 1.0 / 3.6),
        "mph" => ("speed", 0.44704),
        "knot" | "knots" | "kn" => ("speed", 0.514444),
        "b" | "byte" | "bytes" => ("data", 1.0),
        "kb" => ("data", 1000.0),
        "mb" => ("data", 1e6),
        "gb" => ("data", 1e9),
        "tb" => ("data", 1e12),
        "kib" => ("data", 1024.0),
        "mib" => ("data", 1048576.0),
        "gib" => ("data", 1073741824.0),
        "s" | "sec" | "second" | "seconds" => ("time", 1.0),
        "min" | "minute" | "minutes" => ("time", 60.0),
        "h" | "hr" | "hour" | "hours" => ("time", 3600.0),
        "day" | "days" => ("time", 86400.0),
        "week" | "weeks" => ("time", 604800.0),
        "m2" | "sqm" => ("area", 1.0),
        "ft2" | "sqft" => ("area", 0.09290304),
        "acre" | "acres" => ("area", 4046.8564224),
        "ha" | "hectare" | "hectares" => ("area", 10000.0),
        "km2" => ("area", 1e6),
        "mi2" => ("area", 2589988.110336),
        "j" | "joule" | "joules" => ("energy", 1.0),
        "kj" => ("energy", 1000.0),
        "cal" => ("energy", 4.184),
        "kcal" => ("energy", 4184.0),
        "kwh" => ("energy", 3.6e6),
        _ => return None,
    })
}

fn to_celsius(x: f64, u: &str) -> Option<f64> {
    Some(match u {
        "c" | "celsius" => x,
        "f" | "fahrenheit" => (x - 32.0) * 5.0 / 9.0,
        "k" | "kelvin" => x - 273.15,
        _ => return None,
    })
}

fn from_celsius(c: f64, u: &str) -> Option<f64> {
    Some(match u {
        "c" | "celsius" => c,
        "f" | "fahrenheit" => c * 9.0 / 5.0 + 32.0,
        "k" | "kelvin" => c + 273.15,
        _ => return None,
    })
}

pub fn convert(x: f64, from: &str, to: &str) -> Result<f64, String> {
    let (f, t) = (from.trim().to_lowercase(), to.trim().to_lowercase());
    if let (Some(c), Some(_)) = (to_celsius(x, &f), from_celsius(0.0, &t)) {
        return Ok(from_celsius(c, &t).unwrap());
    }
    let (Some((da, fa)), Some((db, fb))) = (unit(&f), unit(&t)) else {
        return Err(format!("don't know how to convert {from} to {to}"));
    };
    if da != db {
        return Err(format!("{from} is {da} and {to} is {db}"));
    }
    Ok(x * fa / fb)
}

fn group_thousands(x: f64, decimals: usize) -> String {
    let s = format!("{:.*}", decimals, x.abs());
    let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((s.clone(), String::new()));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}{out}{frac}", if x < 0.0 { "-" } else { "" })
}

fn to_roman(mut n: u32) -> String {
    const T: [(u32, &str); 13] = [(1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"), (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn from_roman(s: &str) -> Option<u32> {
    let val = |c| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let v: Vec<u32> = s.trim().to_uppercase().chars().map(val).collect::<Option<_>>()?;
    let mut total = 0;
    for i in 0..v.len() {
        if i + 1 < v.len() && v[i] < v[i + 1] { total -= v[i] as i64 } else { total += v[i] as i64 }
    }
    (total > 0 && to_roman(total as u32) == s.trim().to_uppercase()).then_some(total as u32)
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

pub fn defs() -> Vec<Action> {
    vec![
        local("calculate", C, "Evaluate arithmetic: + - * / % ^, parentheses, sqrt, round(x, d), min, max…", &[("expression", "string", "e.g. (12.5 * 4) / 3 + sqrt(16)", true)],
            |v| calculate(s(v, "expression")?).map(num), (json!({"expression": "(2 + 3) * 4 ^ 2 / 8 - sqrt(16)"}), "6")),
        local("unit_convert", C, "Convert between units (length, mass, volume, temperature, speed, data, time, area, energy)", &[("value", "number", "Amount", true), ("from", "string", "From unit, e.g. km, lb, F", true), ("to", "string", "To unit, e.g. mi, kg, C", true)],
            |v| { let x = n(v, "value")?; let r = convert(x, s(v, "from")?, s(v, "to")?)?; Ok(format!("{} {} = {} {}", num(x), s(v, "from")?, num((r * 1e6).round() / 1e6), s(v, "to")?)) },
            (json!({"value": 100, "from": "F", "to": "C"}), "= 37.777778 C")),
        local("percent_of", C, "What is P% of X", &[("percent", "number", "Percent", true), ("value", "number", "Value", true)],
            |v| Ok(num(n(v, "percent")? / 100.0 * n(v, "value")?)), (json!({"percent": 15, "value": 80}), "12")),
        local("percent_change", C, "Percent change from an old value to a new one", &[("old", "number", "Old value", true), ("new", "number", "New value", true)],
            |v| { let (a, b) = (n(v, "old")?, n(v, "new")?); if a == 0.0 { return Err("old value is 0".into()); } Ok(format!("{:+.2}%", (b - a) / a.abs() * 100.0)) },
            (json!({"old": 80, "new": 100}), "+25.00%")),
        local("percent_ratio", C, "X is what percent of Y", &[("part", "number", "Part", true), ("whole", "number", "Whole", true)],
            |v| { let w = n(v, "whole")?; if w == 0.0 { return Err("whole is 0".into()); } Ok(format!("{:.2}%", n(v, "part")? / w * 100.0)) },
            (json!({"part": 1, "whole": 8}), "12.50%")),
        local("round_number", C, "Round to a number of decimal places", &[("value", "number", "Value", true), ("decimals", "integer", "Decimal places (default 0)", false)],
            |v| { let d = n_or(v, "decimals", 0.0) as usize; Ok(format!("{:.*}", d, n(v, "value")?)) }, (json!({"value": 1.23456, "decimals": 2}), "1.23")),
        local("number_format", C, "Format a number with thousands separators", &[("value", "number", "Value", true), ("decimals", "integer", "Decimal places (default 2)", false)],
            |v| Ok(group_thousands(n(v, "value")?, n_or(v, "decimals", 2.0) as usize)), (json!({"value": 1234567.891, "decimals": 2}), "1,234,567.89")),
        local("currency_format", C, "Format an amount as currency", &[("amount", "number", "Amount", true), ("currency", "string", "Code like USD, EUR, GBP, INR, JPY", true)],
            |v| {
                let a = n(v, "amount")?; let c = s(v, "currency")?.to_uppercase();
                let (sym, dec) = match c.as_str() { "USD" => ("$", 2), "EUR" => ("€", 2), "GBP" => ("£", 2), "INR" => ("₹", 2), "JPY" => ("¥", 0), "CNY" => ("¥", 2), "KRW" => ("₩", 0), _ => ("", 2) };
                Ok(if sym.is_empty() { format!("{} {c}", group_thousands(a, dec)) } else { format!("{}{sym}{}", if a < 0.0 { "-" } else { "" }, group_thousands(a.abs(), dec)) })
            },
            (json!({"amount": 1234.5, "currency": "EUR"}), "€1,234.50")),
        local("roman_numerals", C, "Convert between numbers and Roman numerals", &[("value", "string", "A number (1–3999) or Roman numeral", true)],
            |v| { let t = s(v, "value")?.trim(); if let Ok(n2) = t.parse::<u32>() { if !(1..=3999).contains(&n2) { return Err("1 to 3999 only".into()); } Ok(to_roman(n2)) } else { from_roman(t).map(|x| x.to_string()).ok_or_else(|| "not a valid Roman numeral".into()) } },
            (json!({"value": "1994"}), "MCMXCIV")),
        local("base_convert", C, "Convert an integer between bases 2–36", &[("value", "string", "Number", true), ("from", "integer", "From base", true), ("to", "integer", "To base", true)],
            |v| {
                let (from, to) = (n(v, "from")? as u32, n(v, "to")? as u32);
                if !(2..=36).contains(&from) || !(2..=36).contains(&to) { return Err("bases 2–36".into()); }
                let mut x = u128::from_str_radix(s(v, "value")?.trim().trim_start_matches("0x").trim_start_matches("0b"), from).map_err(|e| e.to_string())?;
                if x == 0 { return Ok("0".into()); }
                let mut out = Vec::new(); while x > 0 { out.push(std::char::from_digit((x % to as u128) as u32, to).unwrap()); x /= to as u128; }
                Ok(out.iter().rev().collect())
            },
            (json!({"value": "255", "from": 10, "to": 16}), "ff")),
        local("statistics", C, "Mean, median, mode, standard deviation, min, max of numbers", &[("numbers", "array", "Numbers", true)],
            |v| {
                let mut x = numbers(v, "numbers")?; if x.is_empty() { return Err("no numbers".into()); }
                x.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let n2 = x.len() as f64; let mean = x.iter().sum::<f64>() / n2;
                let median = if x.len() % 2 == 1 { x[x.len() / 2] } else { (x[x.len() / 2 - 1] + x[x.len() / 2]) / 2.0 };
                let var = x.iter().map(|y| (y - mean).powi(2)).sum::<f64>() / if x.len() > 1 { n2 - 1.0 } else { 1.0 };
                let mut counts: Vec<(f64, usize)> = Vec::new(); for y in &x { match counts.iter_mut().find(|c| c.0 == *y) { Some(c) => c.1 += 1, None => counts.push((*y, 1)) } }
                let top = counts.iter().map(|c| c.1).max().unwrap_or(1);
                let mode = if top > 1 { counts.iter().filter(|c| c.1 == top).map(|c| num(c.0)).collect::<Vec<_>>().join(", ") } else { "none".into() };
                Ok(format!("count {}\nmean {}\nmedian {}\nmode {mode}\nstdev {:.4}\nmin {}\nmax {}\nsum {}", x.len(), num(mean), num(median), var.sqrt(), num(x[0]), num(x[x.len() - 1]), num(x.iter().sum())))
            },
            (json!({"numbers": [2, 4, 4, 4, 5, 5, 7, 9]}), "median 4.5")),
        local("percentile", C, "The p-th percentile of numbers", &[("numbers", "array", "Numbers", true), ("p", "number", "Percentile 0–100", true)],
            |v| { let mut x = numbers(v, "numbers")?; if x.is_empty() { return Err("no numbers".into()); } x.sort_by(|a, b| a.partial_cmp(b).unwrap()); let p = n(v, "p")?.clamp(0.0, 100.0) / 100.0; let r = p * (x.len() - 1) as f64; let (lo, hi) = (r.floor() as usize, r.ceil() as usize); Ok(num(x[lo] + (x[hi] - x[lo]) * (r - lo as f64))) },
            (json!({"numbers": [1, 2, 3, 4, 5], "p": 50}), "3")),
        local("loan_payment", C, "Monthly payment, total paid and total interest for a loan", &[("principal", "number", "Amount borrowed", true), ("annual_rate", "number", "Annual interest % (e.g. 6.5)", true), ("years", "number", "Term in years", true)],
            |v| {
                let (p, r, y) = (n(v, "principal")?, n(v, "annual_rate")? / 100.0 / 12.0, n(v, "years")?); let months = (y * 12.0).round();
                let pay = if r == 0.0 { p / months } else { p * r / (1.0 - (1.0 + r).powf(-months)) };
                Ok(format!("monthly payment: {}\ntotal paid: {}\ntotal interest: {}", group_thousands(pay, 2), group_thousands(pay * months, 2), group_thousands(pay * months - p, 2)))
            },
            (json!({"principal": 200000, "annual_rate": 6, "years": 30}), "monthly payment: 1,199.10")),
        local("compound_interest", C, "Future value with compound interest and optional monthly contributions", &[("principal", "number", "Starting amount", true), ("annual_rate", "number", "Annual %", true), ("years", "number", "Years", true), ("monthly_contribution", "number", "Added each month (default 0)", false)],
            |v| {
                let (p, r, y, c) = (n(v, "principal")?, n(v, "annual_rate")? / 100.0 / 12.0, n(v, "years")?, n_or(v, "monthly_contribution", 0.0)); let m = (y * 12.0).round();
                let fv = p * (1.0 + r).powf(m) + if r == 0.0 { c * m } else { c * ((1.0 + r).powf(m) - 1.0) / r };
                Ok(format!("future value: {}\ncontributed: {}\ngrowth: {}", group_thousands(fv, 2), group_thousands(p + c * m, 2), group_thousands(fv - p - c * m, 2)))
            },
            (json!({"principal": 1000, "annual_rate": 12, "years": 1}), "future value: 1,126.83")),
        local("tip_split", C, "Tip and per-person share of a bill", &[("bill", "number", "Bill total", true), ("tip_percent", "number", "Tip % (default 15)", false), ("people", "integer", "People (default 1)", false)],
            |v| { let b = n(v, "bill")?; let t = b * n_or(v, "tip_percent", 15.0) / 100.0; let p = n_or(v, "people", 1.0).max(1.0); Ok(format!("tip: {:.2}\ntotal: {:.2}\neach: {:.2}", t, b + t, (b + t) / p)) },
            (json!({"bill": 100, "tip_percent": 20, "people": 4}), "each: 30.00")),
        local("discount_price", C, "Price after a percentage discount (and tax)", &[("price", "number", "Price", true), ("discount_percent", "number", "Discount %", true), ("tax_percent", "number", "Tax % (default 0)", false)],
            |v| { let p = n(v, "price")? * (1.0 - n(v, "discount_percent")? / 100.0); let t = p * n_or(v, "tax_percent", 0.0) / 100.0; Ok(format!("after discount: {:.2}\ntax: {:.2}\ntotal: {:.2}", p, t, p + t)) },
            (json!({"price": 80, "discount_percent": 25}), "after discount: 60.00")),
        local("bmi", C, "Body-mass index from weight (kg) and height (cm) — a rough screening number only", &[("weight_kg", "number", "Weight in kg", true), ("height_cm", "number", "Height in cm", true)],
            |v| { let h = n(v, "height_cm")? / 100.0; if h <= 0.0 { return Err("height must be positive".into()); } let b = n(v, "weight_kg")? / (h * h); Ok(format!("BMI {:.1} ({})", b, if b < 18.5 { "underweight range" } else if b < 25.0 { "healthy range" } else if b < 30.0 { "overweight range" } else { "obesity range" })) },
            (json!({"weight_kg": 70, "height_cm": 175}), "BMI 22.9")),
        local("gcd_lcm", C, "Greatest common divisor and least common multiple", &[("a", "integer", "First", true), ("b", "integer", "Second", true)],
            |v| { let (a, b) = (n(v, "a")?.abs() as u64, n(v, "b")?.abs() as u64); let g = gcd(a, b); Ok(format!("gcd {g}\nlcm {}", if g == 0 { 0 } else { a / g * b })) },
            (json!({"a": 12, "b": 18}), "lcm 36")),
        local("prime_check", C, "Is a number prime (and its smallest factor if not)", &[("n", "integer", "Number", true)],
            |v| { let x = n(v, "n")? as u64; if x < 2 { return Ok("not prime".into()); } let mut d = 2; while d * d <= x { if x.is_multiple_of(d) { return Ok(format!("not prime (divisible by {d})")); } d += 1; } Ok("prime".into()) },
            (json!({"n": 97}), "prime")),
        local("average", C, "Average (mean) of numbers", &[("numbers", "array", "Numbers", true)],
            |v| { let x = numbers(v, "numbers")?; if x.is_empty() { return Err("no numbers".into()); } Ok(num(x.iter().sum::<f64>() / x.len() as f64)) },
            (json!({"numbers": [1, 2, 3, 4]}), "2.5")),
        local("sum", C, "Sum of numbers", &[("numbers", "array", "Numbers", true)], |v| Ok(num(numbers(v, "numbers")?.iter().sum())), (json!({"numbers": [1.5, 2.5]}), "4")),
        local("ratio_scale", C, "Scale a quantity by a ratio (recipes, resizing): value × to / from", &[("value", "number", "Value", true), ("from", "number", "Original basis", true), ("to", "number", "New basis", true)],
            |v| { let f = n(v, "from")?; if f == 0.0 { return Err("from is 0".into()); } Ok(num(n(v, "value")? * n(v, "to")? / f)) },
            (json!({"value": 300, "from": 4, "to": 6}), "450")),
        local("aspect_ratio", C, "Simplify a width:height ratio and fit a new size", &[("width", "number", "Width", true), ("height", "number", "Height", true), ("new_width", "number", "Optional new width", false)],
            |v| { let (w, h) = (n(v, "width")?, n(v, "height")?); let g = gcd(w as u64, h as u64).max(1); let base = format!("{}:{}", w as u64 / g, h as u64 / g); Ok(match n(v, "new_width") { Ok(nw) => format!("{base}\nnew size: {} x {}", num(nw), num((nw * h / w).round())), Err(_) => base }) },
            (json!({"width": 1920, "height": 1080, "new_width": 1280}), "new size: 1280 x 720")),
        local("number_to_words", C, "Write an integer in English words", &[("n", "integer", "Number (up to trillions)", true)],
            |v| {
                fn words(n: u64) -> String {
                    const O: [&str; 20] = ["zero","one","two","three","four","five","six","seven","eight","nine","ten","eleven","twelve","thirteen","fourteen","fifteen","sixteen","seventeen","eighteen","nineteen"];
                    const T: [&str; 10] = ["","","twenty","thirty","forty","fifty","sixty","seventy","eighty","ninety"];
                    match n { 0..=19 => O[n as usize].into(), 20..=99 => format!("{}{}", T[(n / 10) as usize], if !n.is_multiple_of(10) { format!("-{}", O[(n % 10) as usize]) } else { String::new() }), 100..=999 => format!("{} hundred{}", O[(n / 100) as usize], if !n.is_multiple_of(100) { format!(" {}", words(n % 100)) } else { String::new() }), _ => { for (d, name) in [(1_000_000_000_000u64, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1000, "thousand")] { if n >= d { return format!("{} {name}{}", words(n / d), if !n.is_multiple_of(d) { format!(" {}", words(n % d)) } else { String::new() }); } } unreachable!() } }
                }
                let x = n(v, "n")?; Ok(format!("{}{}", if x < 0.0 { "minus " } else { "" }, words(x.abs() as u64)))
            },
            (json!({"n": 1234}), "one thousand two hundred thirty-four")),
        local("break_even", C, "Units needed to break even", &[("fixed_costs", "number", "Fixed costs", true), ("price", "number", "Price per unit", true), ("unit_cost", "number", "Variable cost per unit", true)],
            |v| { let m = n(v, "price")? - n(v, "unit_cost")?; if m <= 0.0 { return Err("price must exceed unit cost".into()); } Ok(format!("{} units", (n(v, "fixed_costs")? / m).ceil())) },
            (json!({"fixed_costs": 1000, "price": 25, "unit_cost": 15}), "100 units")),
        local("sales_tax", C, "Add or remove tax from an amount", &[("amount", "number", "Amount", true), ("rate", "number", "Tax %", true), ("mode", "string", "add (default) or remove", false)],
            |v| { let (a, r) = (n(v, "amount")?, n(v, "rate")? / 100.0); Ok(if s_or(v, "mode", "add") == "remove" { let net = a / (1.0 + r); format!("net {:.2}, tax {:.2}", net, a - net) } else { format!("tax {:.2}, gross {:.2}", a * r, a * (1.0 + r)) }) },
            (json!({"amount": 120, "rate": 20, "mode": "remove"}), "net 100.00, tax 20.00")),
    ]
}
