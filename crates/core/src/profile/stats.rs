// Pure statistics over plain slices, kept separate from Polars so they are
// easy to test.

use serde::Serialize;
use std::collections::HashMap;

// Values commonly used as "missing" codes in numeric data
const SENTINELS: &[f64] = &[-99999.0, -9999.0, -999.0, -99.0, 999.0, 9999.0, 99999.0];
const HISTOGRAM_BINS: usize = 20;
const TOP_VALUES: usize = 10;
const MAX_EXAMPLES: usize = 5;

#[derive(Debug, Clone, Serialize)]
pub struct Bin {
    pub start: f64,
    pub end: f64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct NumericStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub std: f64,
    pub q1: f64,
    pub q3: f64,
    pub skew: f64,
    pub zeros: usize,
    pub negatives: usize,
    pub negative_examples: Vec<f64>,
    // Outside 1.5 × IQR of the quartiles
    pub outliers: usize,
    // Outside 3 × IQR of the quartiles
    pub extreme_outliers: usize,
    pub outlier_examples: Vec<f64>,
    // Values like -999 that are often codes for "missing", with counts
    pub sentinels: Vec<(f64, usize)>,
    pub histogram: Vec<Bin>,
}

// `values` must not contain NaN; it is sorted in place
pub fn numeric_stats(values: &mut [f64]) -> Option<NumericStats> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let (m2, m3) = values.iter().fold((0.0, 0.0), |(m2, m3), v| {
        let d = v - mean;
        (m2 + d * d, m3 + d * d * d)
    });
    let std = if values.len() > 1 {
        (m2 / (n - 1.0)).sqrt()
    } else {
        0.0
    };
    let skew = if m2 > 0.0 {
        (m3 / n) / (m2 / n).powf(1.5)
    } else {
        0.0
    };

    let q1 = quantile(values, 0.25);
    let q3 = quantile(values, 0.75);
    let iqr = q3 - q1;
    let (mut outliers, mut extreme_outliers, mut outlier_examples) = (0, 0, Vec::new());
    if iqr > 0.0 {
        // Report the most extreme values first: walk inwards from both ends
        let (low, high) = (q1 - 1.5 * iqr, q3 + 1.5 * iqr);
        let (xlow, xhigh) = (q1 - 3.0 * iqr, q3 + 3.0 * iqr);
        let mut candidates: Vec<f64> = values
            .iter()
            .copied()
            .filter(|v| *v < low || *v > high)
            .collect();
        outliers = candidates.len();
        extreme_outliers = candidates
            .iter()
            .filter(|v| **v < xlow || **v > xhigh)
            .count();
        let median = quantile(values, 0.5);
        candidates.sort_by(|a, b| (b - median).abs().total_cmp(&(a - median).abs()));
        candidates.dedup();
        outlier_examples = candidates.into_iter().take(MAX_EXAMPLES).collect();
    }

    let mut negative_examples: Vec<f64> = values.iter().copied().filter(|v| *v < 0.0).collect();
    let negatives = negative_examples.len();
    negative_examples.dedup();
    negative_examples.truncate(MAX_EXAMPLES);

    // Only codes that stand apart from the rest of the data count: 999 in a
    // column of values from 0 to 1000 is just a value.
    let outside_typical = |v: f64| {
        if iqr > 0.0 {
            v < q1 - 1.5 * iqr || v > q3 + 1.5 * iqr
        } else {
            v != q1
        }
    };
    let sentinels = SENTINELS
        .iter()
        .filter(|s| outside_typical(**s))
        .filter_map(|s| {
            let count = values.iter().filter(|v| *v == s).count();
            (count > 0).then_some((*s, count))
        })
        .collect();

    Some(NumericStats {
        min: values[0],
        max: values[values.len() - 1],
        mean,
        median: quantile(values, 0.5),
        std,
        q1,
        q3,
        skew,
        zeros: values.iter().filter(|v| **v == 0.0).count(),
        negatives,
        negative_examples,
        outliers,
        extreme_outliers,
        outlier_examples,
        sentinels,
        histogram: histogram(values),
    })
}

// Linear-interpolated quantile of sorted values
pub fn quantile(sorted: &[f64], p: f64) -> f64 {
    let pos = p * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

// Equal-width histogram of sorted values. Small integer ranges get one bin per value.
fn histogram(sorted: &[f64]) -> Vec<Bin> {
    let (min, max) = (sorted[0], sorted[sorted.len() - 1]);
    if min == max {
        return vec![Bin {
            start: min,
            end: max,
            count: sorted.len(),
        }];
    }
    let integral = sorted.iter().all(|v| v.fract() == 0.0);
    let span = max - min;
    let (bins, width) = if integral && span < HISTOGRAM_BINS as f64 {
        (span as usize + 1, 1.0)
    } else {
        (HISTOGRAM_BINS, span / HISTOGRAM_BINS as f64)
    };
    let mut counts = vec![0; bins];
    for v in sorted {
        let i = (((v - min) / width) as usize).min(bins - 1);
        counts[i] += 1;
    }
    counts
        .into_iter()
        .enumerate()
        .map(|(i, count)| Bin {
            start: min + i as f64 * width,
            end: if integral && width == 1.0 {
                min + i as f64
            } else {
                min + (i + 1) as f64 * width
            },
            count,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoricalStats {
    pub distinct: usize,
    // Most frequent values with counts, most common first
    pub top: Vec<(String, usize)>,
    // Values that occur exactly once
    pub singletons: usize,
    // Groups of values that differ only by case or spacing, e.g. ["Basic", "basic"]
    pub inconsistent_variants: Vec<Vec<String>>,
    pub min_length: usize,
    pub max_length: usize,
    pub mean_length: f64,
}

pub fn categorical_stats<'a>(values: impl Iterator<Item = &'a str>) -> Option<CategoricalStats> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let (mut total_len, mut n, mut min_length, mut max_length) = (0usize, 0usize, usize::MAX, 0);
    for v in values {
        *counts.entry(v).or_default() += 1;
        let len = v.chars().count();
        total_len += len;
        min_length = min_length.min(len);
        max_length = max_length.max(len);
        n += 1;
    }
    if n == 0 {
        return None;
    }

    let mut by_key: HashMap<String, Vec<&str>> = HashMap::new();
    for v in counts.keys() {
        by_key.entry(normalize(v)).or_default().push(v);
    }
    let mut inconsistent_variants: Vec<Vec<String>> = by_key
        .into_values()
        .filter(|variants| variants.len() > 1)
        .map(|mut variants| {
            // Most common spelling first
            variants.sort_by_key(|v| (std::cmp::Reverse(counts[v]), *v));
            variants.into_iter().map(String::from).collect()
        })
        .collect();
    inconsistent_variants.sort();
    inconsistent_variants.truncate(MAX_EXAMPLES);

    let mut top: Vec<(&str, usize)> = counts.iter().map(|(k, v)| (*k, *v)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    Some(CategoricalStats {
        distinct: counts.len(),
        singletons: counts.values().filter(|c| **c == 1).count(),
        top: top
            .into_iter()
            .take(TOP_VALUES)
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        inconsistent_variants,
        min_length,
        max_length,
        mean_length: total_len as f64 / n as f64,
    })
}

// Lowercase and collapse internal whitespace, so "New  York" ~ "new york"
fn normalize(v: &str) -> String {
    v.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// Pearson correlation of paired values
pub fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len();
    if n < 3 || n != ys.len() {
        return None;
    }
    let mx = xs.iter().sum::<f64>() / n as f64;
    let my = ys.iter().sum::<f64>() / n as f64;
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in xs.iter().zip(ys) {
        let (dx, dy) = (x - mx, y - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx == 0.0 || syy == 0.0 {
        return None;
    }
    Some(sxy / (sxx * syy).sqrt())
}

// Ranks (1-based, ties get their average rank) of possibly-missing values
pub fn ranks(values: &[Option<f64>]) -> Vec<Option<f64>> {
    let mut order: Vec<(usize, f64)> = values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|v| (i, v)))
        .collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut out = vec![None; values.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && order[j + 1].1 == order[i].1 {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 + 1.0;
        for item in &order[i..=j] {
            out[item.0] = Some(rank);
        }
        i = j + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_numeric_stats() {
        let mut values = vec![4.0, 1.0, 3.0, 2.0, 5.0];
        let s = numeric_stats(&mut values).unwrap();
        assert_eq!((s.min, s.max, s.mean, s.median), (1.0, 5.0, 3.0, 3.0));
        assert_eq!((s.q1, s.q3), (2.0, 4.0));
        assert!((s.std - 1.5811).abs() < 1e-4);
        assert_eq!(s.skew, 0.0);
        assert_eq!(s.outliers, 0);
        assert_eq!(s.histogram.len(), 5);
        assert!(s.histogram.iter().all(|b| b.count == 1));
    }

    #[test]
    fn outliers_negatives_and_sentinels() {
        let mut values: Vec<f64> = (1..=20).map(|v| v as f64).collect();
        values.extend([-999.0, 500.0, -1.0]);
        let s = numeric_stats(&mut values).unwrap();
        assert_eq!(s.outliers, 2);
        assert_eq!(s.extreme_outliers, 2);
        assert_eq!(s.outlier_examples, vec![-999.0, 500.0]);
        assert_eq!(s.negatives, 2);
        assert_eq!(s.sentinels, vec![(-999.0, 1)]);
        assert!(s.skew < 0.0);
    }

    #[test]
    fn sentinel_codes_only_when_they_stand_out() {
        let mut values: Vec<f64> = (0..=1000).map(|v| v as f64).collect();
        assert!(numeric_stats(&mut values).unwrap().sentinels.is_empty());

        let mut values = vec![5.0; 50];
        values.push(9999.0);
        assert_eq!(
            numeric_stats(&mut values).unwrap().sentinels,
            vec![(9999.0, 1)]
        );
    }

    #[test]
    fn histogram_bins_cover_all_values() {
        let mut values: Vec<f64> = (0..1000).map(|v| v as f64 * 0.37).collect();
        let s = numeric_stats(&mut values).unwrap();
        assert_eq!(s.histogram.len(), 20);
        assert_eq!(s.histogram.iter().map(|b| b.count).sum::<usize>(), 1000);
    }

    #[test]
    fn categorical_counts_and_variants() {
        let values = [
            "Basic",
            "basic",
            "Pro",
            "Basic",
            "New York",
            "new  york",
            "Pro",
        ];
        let s = categorical_stats(values.into_iter()).unwrap();
        assert_eq!(s.distinct, 5);
        assert_eq!(s.top[0], ("Basic".to_string(), 2));
        assert_eq!(s.singletons, 3);
        assert_eq!(
            s.inconsistent_variants,
            vec![
                vec!["Basic".to_string(), "basic".to_string()],
                vec!["New York".to_string(), "new  york".to_string()],
            ]
        );
    }

    #[test]
    fn correlation_and_ranks() {
        let xs = [1.0, 2.0, 3.0, 4.0];
        assert!((pearson(&xs, &[2.0, 4.0, 6.0, 8.0]).unwrap() - 1.0).abs() < 1e-12);
        assert!((pearson(&xs, &[8.0, 6.0, 4.0, 2.0]).unwrap() + 1.0).abs() < 1e-12);
        assert_eq!(pearson(&xs, &[1.0, 1.0, 1.0, 1.0]), None);
        assert_eq!(
            ranks(&[Some(10.0), None, Some(5.0), Some(10.0)]),
            vec![Some(2.5), None, Some(1.0), Some(2.5)]
        );
    }
}
