//! Weather service contract — forecasts, locations, alerts.

use serde::{Deserialize, Serialize};
use crate::email::ServiceError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentWeather {
    pub temperature: f64,
    pub feels_like: f64,
    pub humidity: i32,
    pub wind_speed: f64,
    pub wind_direction: String,
    pub condition: String,
    pub icon: String,
    pub uv_index: f64,
    pub visibility_km: f64,
    pub pressure_hpa: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourlyForecast {
    pub time: String,
    pub temperature: f64,
    pub condition: String,
    pub icon: String,
    pub precipitation_chance: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyForecast {
    pub date: String,
    pub temp_high: f64,
    pub temp_low: f64,
    pub condition: String,
    pub icon: String,
    pub precipitation_chance: i32,
    pub sunrise: String,
    pub sunset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherAlert {
    pub severity: String,
    pub title: String,
    pub description: String,
    pub expires: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirQuality {
    pub value: f64,
    pub label: String,
    pub level: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Weather service operations.
pub trait WeatherService: Send + Sync {
    fn current(&self, location: &Location) -> Result<CurrentWeather, ServiceError>;
    fn hourly(&self, location: &Location, hours: u32) -> Result<Vec<HourlyForecast>, ServiceError>;
    fn daily(&self, location: &Location, days: u32) -> Result<Vec<DailyForecast>, ServiceError>;
    fn alerts(&self, location: &Location) -> Result<Vec<WeatherAlert>, ServiceError>;
    fn air_quality(&self, location: &Location) -> Result<AirQuality, ServiceError>;
}
