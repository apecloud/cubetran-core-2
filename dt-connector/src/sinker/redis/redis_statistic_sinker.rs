use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use dt_common::error::Error;
use dt_common::log_statistic;
use dt_common::monitor::monitor::Monitor;
use dt_meta::dt_data::DtData;
use dt_meta::redis::redis_statistic_type::RedisStatisticType;
use serde::Serialize;
use serde_json::json;

use crate::Sinker;

pub struct RedisStatisticSinker {
    pub statistic_type: RedisStatisticType,
    pub monitor: Arc<Mutex<Monitor>>,
    pub data_size_threshold: usize,
    pub freq_threshold: i64,
}

#[derive(Serialize)]
struct BigKeyInfo {
    db_id: i64,
    key_type: String,
    key: String,
    data_size: usize,
}

#[derive(Serialize)]
struct HotKeyInfo {
    db_id: i64,
    key: String,
    freq: i64,
}

#[async_trait]
impl Sinker for RedisStatisticSinker {
    async fn sink_raw(&mut self, mut data: Vec<DtData>, _batch: bool) -> Result<(), Error> {
        for dt_data in data.iter_mut() {
            if let DtData::Redis { entry } = dt_data {
                match self.statistic_type {
                    RedisStatisticType::BigKey => {
                        if entry.get_data_malloc_size() < self.data_size_threshold {
                            continue;
                        }
                        let info = BigKeyInfo {
                            db_id: entry.db_id,
                            key_type: entry.get_type(),
                            key: entry.key.to_string(),
                            data_size: entry.get_data_malloc_size(),
                        };
                        log_statistic!("{}", json!(info).to_string());
                    }

                    RedisStatisticType::HotKey => {
                        if entry.freq < self.freq_threshold {
                            continue;
                        }
                        let info = HotKeyInfo {
                            db_id: entry.db_id,
                            key: entry.key.to_string(),
                            freq: entry.freq,
                        };
                        log_statistic!("{}", json!(info).to_string());
                    }
                }
            }
        }
        Ok(())
    }
}
