use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::bail;
use async_trait::async_trait;
use rdkafka::producer::{FutureProducer, FutureRecord};

use crate::{rdb_router::RdbRouter, sinker::base_sinker::BaseSinker, Sinker};

use dt_common::monitor::monitor::Monitor;

use dt_common::meta::{avro::avro_converter::AvroConverter, row_data::RowData};

pub struct RdkafkaSinker {
    pub batch_size: usize,
    pub router: RdbRouter,
    pub producer: FutureProducer,
    pub avro_converter: AvroConverter,
    pub monitor: Arc<Mutex<Monitor>>,
    pub queue_timeout_secs: u64,
}

#[async_trait]
impl Sinker for RdkafkaSinker {
    async fn sink_dml(&mut self, data: Vec<RowData>, _batch: bool) -> anyhow::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        self.send_avro(data).await
    }
}

impl RdkafkaSinker {
    async fn send_avro(&mut self, data: Vec<RowData>) -> anyhow::Result<()> {
        let start_time = Instant::now();
        let batch_size = data.len();
        let mut data_size = 0;

        let producer = &self.producer.clone();
        let queue_timeout = Duration::from_secs(self.queue_timeout_secs);
        let mut futures = Vec::new();

        // This loop is non blocking: all messages will be sent one after the other, without waiting
        // for the results.
        for mut row_data in data {
            data_size += row_data.data_size;
            row_data.convert_raw_string();
            let topic = self.router.get_topic(&row_data.schema, &row_data.tb);
            let key = self.avro_converter.row_data_to_avro_key(&row_data).await?;
            let payload = self.avro_converter.row_data_to_avro_value(row_data).await?;

            // The send operation on the topic returns a future, which will be
            // completed once the result or failure from Kafka is received.
            let delivery_status = async move {
                producer
                    .send(
                        FutureRecord::to(topic).payload(&payload).key(&key),
                        queue_timeout,
                    )
                    .await
            };
            futures.push(delivery_status);
        }

        // This loop will wait until all delivery statuses have been received.
        for future in futures {
            if let Err(err) = future.await {
                bail!(format!("failed in kafka producer, error: {:?}", err));
            }
        }

        BaseSinker::update_batch_monitor(&mut self.monitor, batch_size, data_size, start_time)
    }
}