//! Line diff (Myers O(ND)) producing hunks with context, for comparing file
//! versions without Git.

use crate::json::Json;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Eq,
    Del,
    Add,
}

pub const MAX_EDIT_DISTANCE: usize = 4000;

/// Returns the edit script as (op, old_index, new_index) per line, or None if
/// the files differ by more than MAX_EDIT_DISTANCE lines.
pub fn diff_lines<'a>(a: &[&'a str], b: &[&'a str]) -> Option<Vec<(Op, usize, usize)>> {
    // trim common prefix/suffix first (cheap, and keeps the Myers part small)
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mid = myers(am, bm)?;
    let mut out: Vec<(Op, usize, usize)> = (0..pre).map(|i| (Op::Eq, i, i)).collect();
    out.extend(mid.into_iter().map(|(o, i, j)| (o, i + pre, j + pre)));
    for k in 0..suf {
        out.push((Op::Eq, a.len() - suf + k, b.len() - suf + k));
    }
    Some(out)
}

fn myers(a: &[&str], b: &[&str]) -> Option<Vec<(Op, usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    if n == 0 {
        return Some((0..b.len()).map(|j| (Op::Add, 0, j)).collect());
    }
    if m == 0 {
        return Some((0..a.len()).map(|i| (Op::Del, i, 0)).collect());
    }
    let max = ((n + m) as usize).min(MAX_EDIT_DISTANCE) as isize;
    let off = max + 1;
    let mut v = vec![0isize; (2 * max + 3) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    'outer: for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let ki = (k + off) as usize;
            let mut x = if k == -d || (k != d && v[ki - 1] < v[ki + 1]) { v[ki + 1] } else { v[ki - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[ki] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'outer;
            }
            k += 2;
        }
    }
    let dfinal = found?;
    // backtrack
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=dfinal).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let ki = (k + off) as usize;
        let prev_k = if k == -d || (k != d && v[ki - 1] < v[ki + 1]) { k + 1 } else { k - 1 };
        let prev_x = v[(prev_k + off) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            ops.push((Op::Eq, x as usize, y as usize));
        }
        if x == prev_x {
            y -= 1;
            ops.push((Op::Add, x as usize, y as usize));
        } else {
            x -= 1;
            ops.push((Op::Del, x as usize, y as usize));
        }
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        ops.push((Op::Eq, x as usize, y as usize));
    }
    ops.reverse();
    Some(ops)
}

/// Text diff as JSON hunks with `context` unchanged lines around changes:
/// {identical, tooLarge, added, removed, hunks:[{oldStart,newStart,lines:[{t,text,o,n}]}]}
/// Line numbers are 1-based.
pub fn diff_json(old: &str, new: &str, context: usize) -> Json {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    if a == b {
        return Json::obj().with("identical", true).with("tooLarge", false).with("added", 0).with("removed", 0).with("hunks", Json::Arr(vec![]));
    }
    let Some(ops) = diff_lines(&a, &b) else {
        return Json::obj().with("identical", false).with("tooLarge", true).with("added", Json::Null).with("removed", Json::Null).with("hunks", Json::Arr(vec![]));
    };
    let added = ops.iter().filter(|o| o.0 == Op::Add).count();
    let removed = ops.iter().filter(|o| o.0 == Op::Del).count();
    // group into hunks
    let changed: Vec<usize> = ops.iter().enumerate().filter(|(_, o)| o.0 != Op::Eq).map(|(i, _)| i).collect();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for &i in &changed {
        let s = i.saturating_sub(context);
        let e = (i + context + 1).min(ops.len());
        match ranges.last_mut() {
            Some(r) if s <= r.1 => r.1 = r.1.max(e),
            _ => ranges.push((s, e)),
        }
    }
    let hunks: Vec<Json> = ranges
        .iter()
        .map(|&(s, e)| {
            let lines: Vec<Json> = ops[s..e]
                .iter()
                .map(|&(op, i, j)| match op {
                    Op::Eq => Json::obj().with("t", " ").with("text", a[i]).with("o", i + 1).with("n", j + 1),
                    Op::Del => Json::obj().with("t", "-").with("text", a[i]).with("o", i + 1).with("n", Json::Null),
                    Op::Add => Json::obj().with("t", "+").with("text", b[j]).with("o", Json::Null).with("n", j + 1),
                })
                .collect();
            Json::obj().with("lines", Json::Arr(lines))
        })
        .collect();
    Json::obj().with("identical", false).with("tooLarge", false).with("added", added).with("removed", removed).with("hunks", Json::Arr(hunks))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn apply(a: &[&str], b: &[&str]) -> Vec<String> {
        // reconstruct b from the script to validate correctness
        let ops = diff_lines(a, b).unwrap();
        let mut out = Vec::new();
        for (op, _i, j) in ops {
            match op {
                Op::Eq | Op::Add => out.push(b[j].to_string()),
                Op::Del => {}
            }
        }
        out
    }
    #[test]
    fn myers_correct_and_minimal() {
        let cases: Vec<(&str, &str)> = vec![
            ("a\nb\nc", "a\nb\nc"),
            ("", "x\ny"),
            ("x\ny", ""),
            ("a\nb\nc\nd", "a\nx\nc\nd\ne"),
            ("init()\n{\n  a();\n}\n", "init()\n{\n  a();\n  b();\n}\n"),
            ("1\n2\n3\n4\n5\n6", "6\n5\n4\n3\n2\n1"),
        ];
        for (x, y) in cases {
            let a: Vec<&str> = x.lines().collect();
            let b: Vec<&str> = y.lines().collect();
            assert_eq!(apply(&a, &b), b.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{x:?} -> {y:?}");
        }
        let j = diff_json("a\nb\nc\nd\ne\nf\ng\nh\ni\nj", "a\nb\nc\nd\nE\nf\ng\nh\ni\nj", 1);
        assert_eq!(j.get("added").unwrap().as_i64(), Some(1));
        assert_eq!(j.get("removed").unwrap().as_i64(), Some(1));
        let h = j.get("hunks").unwrap().as_arr().unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].get("lines").unwrap().as_arr().unwrap().len(), 4); // d, -e, +E, f
        // randomized check
        let mut seed = 12345u64;
        let mut rnd = || { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; seed };
        for _ in 0..200 {
            let a: Vec<String> = (0..(rnd() % 30)).map(|_| format!("l{}", rnd() % 6)).collect();
            let b: Vec<String> = (0..(rnd() % 30)).map(|_| format!("l{}", rnd() % 6)).collect();
            let ar: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
            let br: Vec<&str> = b.iter().map(|s| s.as_str()).collect();
            assert_eq!(apply(&ar, &br), b);
        }
    }
}
