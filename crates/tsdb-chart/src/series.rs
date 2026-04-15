use tsdb_types::model::Timestamp;

#[derive(Debug, Clone)]
pub struct TimeSeries {
    pub name: String,
    pub timestamps: Vec<Timestamp>,
    pub values: Vec<f64>,
}

impl TimeSeries {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            timestamps: Vec::new(),
            values: Vec::new(),
        }
    }

    pub fn add_point(&mut self, ts: Timestamp, value: f64) {
        self.timestamps.push(ts);
        self.values.push(value);
    }

    pub fn from_pairs(name: impl Into<String>, pairs: Vec<(Timestamp, f64)>) -> Self {
        let mut timestamps = Vec::with_capacity(pairs.len());
        let mut values = Vec::with_capacity(pairs.len());
        for (ts, v) in pairs {
            timestamps.push(ts);
            values.push(v);
        }
        Self {
            name: name.into(),
            timestamps,
            values,
        }
    }

    pub fn min_value(&self) -> f64 {
        self.values.iter().copied().fold(f64::INFINITY, f64::min)
    }

    pub fn max_value(&self) -> f64 {
        self.values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    }

    pub fn avg_value(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        self.values.iter().sum::<f64>() / self.values.len() as f64
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn downsample(&self, max_points: usize) -> TimeSeries {
        if self.len() <= max_points {
            return self.clone();
        }

        let step = self.len() as f64 / max_points as f64;
        let mut new_ts = Vec::with_capacity(max_points);
        let mut new_vals = Vec::with_capacity(max_points);

        for i in 0..max_points {
            let start = (i as f64 * step) as usize;
            let end = ((i + 1) as f64 * step).min(self.len() as f64) as usize;

            if start >= end {
                continue;
            }

            let avg: f64 = self.values[start..end].iter().sum::<f64>() / (end - start) as f64;
            let mid_ts = self.timestamps[(start + end) / 2];
            new_ts.push(mid_ts);
            new_vals.push(avg);
        }

        TimeSeries {
            name: self.name.clone(),
            timestamps: new_ts,
            values: new_vals,
        }
    }
}
