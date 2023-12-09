use std::{sync::Arc, time::Instant};

use async_rwlock::RwLock;
use dt_common::{
    error::Error,
    monitor::monitor::{CounterType, Monitor},
};

pub struct BaseSinker {}

impl BaseSinker {
    pub async fn update_batch_monitor(
        monitor: &mut Arc<RwLock<Monitor>>,
        batch_size: usize,
        start_time: Instant,
    ) -> Result<(), Error> {
        monitor
            .write()
            .await
            .add_counter(CounterType::RecordsPerQuery, batch_size)
            .add_counter(CounterType::Records, batch_size)
            .add_counter(
                CounterType::RtPerQuery,
                start_time.elapsed().as_micros() as usize,
            );
        Ok(())
    }

    pub async fn update_serial_monitor(
        monitor: &mut Arc<RwLock<Monitor>>,
        record_count: usize,
        start_time: Instant,
    ) -> Result<(), Error> {
        monitor
            .write()
            .await
            .add_batch_counter(CounterType::RecordsPerQuery, record_count, record_count)
            .add_counter(CounterType::Records, record_count)
            .add_counter(CounterType::SerialWrites, record_count)
            .add_batch_counter(
                CounterType::RtPerQuery,
                start_time.elapsed().as_micros() as usize,
                record_count,
            );
        Ok(())
    }
}

#[macro_export(local_inner_macros)]
macro_rules! call_batch_fn {
    ($self:ident, $data:ident, $batch_fn:expr) => {
        let all_count = $data.len();
        let mut sinked_count = 0;

        loop {
            let mut batch_size = $self.batch_size;
            if all_count - sinked_count < batch_size {
                batch_size = all_count - sinked_count;
            }

            if batch_size == 0 {
                break;
            }

            $batch_fn($self, &mut $data, sinked_count, batch_size)
                .await
                .unwrap();

            sinked_count += batch_size;
        }
    };
}

#[macro_export(local_inner_macros)]
macro_rules! close_conn_pool {
    ($self:ident) => {
        if $self.conn_pool.is_closed() {
            Ok(())
        } else {
            Ok($self.conn_pool.close().await)
        }
    };
}
