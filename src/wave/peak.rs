#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Peak {
    time: f64,
    frequency: f64,
    amplitude: f64,
}

impl Peak {
    pub fn new(x: f64, y: f64, val: f64) -> Peak {
        Self {
            time: x,
            frequency: y,
            amplitude: val,
        }
    }

    pub fn amplitude(&self) -> f64 {
        self.amplitude
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn frequency(&self) -> f64 {
        self.frequency
    }

    pub fn distance(&self, other: Peak) -> f64 {
        let dx = self.time() - other.time();
        let dy = self.frequency() - other.frequency();
        let distance = ((dx * dx) + (dy * dy)).sqrt();
        distance
    }
}
