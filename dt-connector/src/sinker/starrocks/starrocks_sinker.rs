use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::{call_batch_fn, sinker::base_sinker::BaseSinker, Sinker};
use anyhow::bail;
use async_trait::async_trait;
use dt_common::meta::{row_data::RowData, row_type::RowType};
use dt_common::{error::Error, log_error, monitor::monitor::Monitor};
use reqwest::{header, Client, Method, Response, StatusCode};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct StarRocksSinker {
    pub batch_size: usize,
    pub client: Client,
    pub host: String,
    pub port: String,
    pub username: String,
    pub password: String,
    pub monitor: Arc<Mutex<Monitor>>,
}

#[async_trait]
impl Sinker for StarRocksSinker {
    async fn sink_dml(&mut self, mut data: Vec<RowData>, batch: bool) -> anyhow::Result<()> {
        if data.is_empty() {
            return Ok(());
        }

        if !batch {
            self.serial_sink(data).await.unwrap();
        } else {
            match data[0].row_type {
                RowType::Insert | RowType::Delete => {
                    call_batch_fn!(self, data, Self::batch_sink);
                }
                _ => self.serial_sink(data).await.unwrap(),
            }
        }
        Ok(())
    }
}

impl StarRocksSinker {
    async fn serial_sink(&mut self, mut data: Vec<RowData>) -> anyhow::Result<()> {
        let start_time = Instant::now();
        let mut data_size = 0;

        let data = data.as_mut_slice();
        for i in 0..data.len() {
            data_size += data[i].data_size;
            self.send_data(data, i, 1).await.unwrap();
        }

        BaseSinker::update_serial_monitor(&mut self.monitor, data.len(), data_size, start_time)
            .await
    }

    async fn batch_sink(
        &mut self,
        data: &mut [RowData],
        start_index: usize,
        batch_size: usize,
    ) -> anyhow::Result<()> {
        let start_time = Instant::now();

        let data_size = self.send_data(data, start_index, batch_size).await.unwrap();

        BaseSinker::update_batch_monitor(&mut self.monitor, batch_size, data_size, start_time).await
    }

    async fn send_data(
        &self,
        data: &mut [RowData],
        start_index: usize,
        batch_size: usize,
    ) -> anyhow::Result<usize> {
        let mut data_size = 0;
        // build stream load data
        let mut load_data = Vec::new();
        for rd in data.iter().skip(start_index).take(batch_size) {
            data_size += rd.data_size;
            if data[start_index].row_type == RowType::Delete {
                load_data.push(rd.before.as_ref().unwrap());
            } else {
                load_data.push(rd.after.as_ref().unwrap());
            }
        }
        let body = json!(load_data).to_string();
        let db = &data[start_index].schema;
        let tb = &data[start_index].tb;
        let op = if data[start_index].row_type == RowType::Delete {
            "delete"
        } else {
            ""
        };

        // do stream load
        let url = format!(
            "http://{}:{}/api/{}/{}/_stream_load",
            self.host, self.port, db, tb
        );
        let request = self.build_request(&url, op, &body).unwrap();
        let response = self.client.execute(request).await.unwrap();
        Self::check_response(response).await.unwrap();

        Ok(data_size)
    }

    fn build_request(&self, url: &str, op: &str, body: &str) -> anyhow::Result<reqwest::Request> {
        let password = if self.password.is_empty() {
            None
        } else {
            Some(self.password.clone())
        };

        // https://docs.starrocks.io/en-us/2.5/loading/Load_to_Primary_Key_tables
        let mut put = self
            .client
            .request(Method::PUT, url)
            .basic_auth(&self.username, password)
            .header(header::EXPECT, "100-continue")
            .header("format", "json")
            .header("strip_outer_array", "true")
            .body(body.to_string());
        // by default, the __op will be upsert
        if !op.is_empty() {
            let op = format!("__op='{}'", op);
            put = put.header("columns", op);
        }
        Ok(put.build()?)
    }

    async fn check_response(response: Response) -> anyhow::Result<()> {
        let status_code = response.status();
        if status_code != StatusCode::OK {
            bail! {Error::HttpError(format!(
                "stream load request failed, status_code: {}",
                status_code
            ))}
        }

        // response example:
        // {
        //     "TxnId": 2039,
        //     "Label": "54afc14e-9088-46df-b532-4deaf4437b42",
        //     "Status": "Success",
        //     "Message": "OK",
        //     "NumberTotalRows": 3,
        //     "NumberLoadedRows": 3,
        //     "NumberFilteredRows": 0,
        //     "NumberUnselectedRows": 0,
        //     "LoadBytes": 221,
        //     "LoadTimeMs": 228,
        //     "BeginTxnTimeMs": 34,
        //     "StreamLoadPlanTimeMs": 48,
        //     "ReadDataTimeMs": 0,
        //     "WriteDataTimeMs": 107,
        //     "CommitAndPublishTimeMs": 36
        // }
        let load_result = &response.text().await.unwrap();
        let json_value: Value = serde_json::from_str(load_result).unwrap();
        if json_value["Status"] != "Success" {
            let err = format!(
                "stream load request failed, status_code: {}, load_result: {}",
                status_code, load_result,
            );
            log_error!("{}", err);
            bail! {Error::HttpError(err)}
        }
        Ok(())
    }
}
