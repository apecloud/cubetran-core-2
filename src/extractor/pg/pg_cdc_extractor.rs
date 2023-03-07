use std::{
    collections::HashMap,
    sync::atomic::AtomicBool,
    time::{Duration, UNIX_EPOCH},
};

use async_trait::async_trait;

use concurrent_queue::ConcurrentQueue;
use futures::StreamExt;

use postgres_protocol::message::backend::{
    DeleteBody, InsertBody,
    LogicalReplicationMessage::{
        Begin, Commit, Delete, Insert, Origin, Relation, Truncate, Type, Update,
    },
    RelationBody,
    ReplicationMessage::*,
    TupleData, UpdateBody,
};

use postgres_types::PgLsn;

use crate::{
    error::Error,
    extractor::{pg::pg_cdc_client::PgCdcClient, rdb_filter::RdbFilter},
    meta::{
        col_value::ColValue,
        pg::{pg_meta_manager::PgMetaManager, pg_tb_meta::PgTbMeta},
        row_data::RowData,
        row_type::RowType,
    },
    task::task_util::TaskUtil,
    traits::Extractor,
};

use super::pg_col_value_convertor::PgColValueConvertor;

pub struct PgCdcExtractor<'a> {
    pub meta_manager: PgMetaManager,
    pub buffer: &'a ConcurrentQueue<RowData>,
    pub filter: RdbFilter,
    pub url: String,
    pub slot_name: String,
    pub start_sln: String,
    pub shut_down: &'a AtomicBool,
}

#[async_trait]
impl Extractor for PgCdcExtractor<'_> {
    async fn extract(&mut self) -> Result<(), Error> {
        Ok(self.extract_internal().await.unwrap())
    }

    async fn close(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl PgCdcExtractor<'_> {
    pub async fn extract_internal(&mut self) -> Result<(), Error> {
        let mut cdc_client = PgCdcClient {
            url: self.url.clone(),
            slot_name: self.slot_name.clone(),
            start_sln: self.start_sln.clone(),
        };
        let stream = cdc_client.connect().await?;
        tokio::pin!(stream);

        // refer: https://www.postgresql.org/docs/10/protocol-replication.html to get WAL data details
        loop {
            match stream.next().await {
                Some(Ok(XLogData(body))) => {
                    let data = body.into_data();
                    match data {
                        Relation(relation) => {
                            self.decode_relation(&relation).await?;
                        }

                        Begin(_begin) => {}

                        Commit(_commit) => {}

                        Origin(_origin) => {}

                        Truncate(_truncate) => {}

                        Type(_typee) => {}

                        Insert(insert) => {
                            self.decode_insert(&insert).await?;
                        }

                        Update(update) => {
                            self.decode_update(&update).await?;
                        }

                        Delete(delete) => {
                            self.decode_delete(&delete).await?;
                        }

                        _ => {
                            println!("other XLogData: {:?}", data);
                        }
                    }
                }

                Some(Ok(PrimaryKeepAlive(data))) => {
                    // Send a standby status update and require a keep alive response
                    if data.reply() == 1 {
                        // TODO, response with sinked sln
                        let lsn: PgLsn = "0/2C280E70".parse().unwrap();
                        // Postgres epoch is 2000-01-01T00:00:00Z
                        let pg_epoch = UNIX_EPOCH + Duration::from_secs(946_684_800);
                        let ts = pg_epoch.elapsed().unwrap().as_micros() as i64;
                        stream
                            .as_mut()
                            .standby_status_update(lsn, lsn, lsn, ts, 1)
                            .await
                            .unwrap();
                    }
                }

                Some(Ok(data)) => {
                    println!("unkown replication data: {:?}", data);
                }

                Some(Err(x)) => panic!("unexpected replication stream error: {}", x),

                None => panic!("unexpected replication stream end"),
            }
        }
    }

    async fn decode_relation(&mut self, event: &RelationBody) -> Result<(), Error> {
        // todo, use event.rel_id()
        let mut tb_meta = self
            .meta_manager
            .get_tb_meta(event.namespace()?, event.name()?)
            .await?;

        let mut col_names = Vec::new();
        for column in event.columns() {
            // todo: check type_id in oid_to_type
            let col_type = self
                .meta_manager
                .type_registry
                .oid_to_type
                .get(&column.type_id())
                .unwrap();
            let col_name = column.name()?;
            // update meta
            tb_meta
                .col_type_map
                .insert(col_name.to_string(), col_type.clone());

            col_names.push(col_name.to_string());
        }

        // align the column order of tb_meta to that of the wal log
        tb_meta.cols = col_names;
        self.meta_manager
            .update_tb_meta_by_oid(event.rel_id() as i32, tb_meta)?;
        Ok(())
    }

    async fn decode_insert(&mut self, event: &InsertBody) -> Result<(), Error> {
        let tb_meta = self
            .meta_manager
            .get_tb_meta_by_oid(event.rel_id() as i32)?;
        let col_values = self.parse_row_data(&tb_meta, event.tuple().tuple_data())?;

        let row_data = RowData {
            db: tb_meta.schema,
            tb: tb_meta.tb,
            row_type: RowType::Insert,
            before: Option::None,
            after: Some(col_values),
            position: "".to_string(),
        };
        self.push_row_to_buf(row_data).await
    }

    async fn decode_update(&mut self, event: &UpdateBody) -> Result<(), Error> {
        let tb_meta = self
            .meta_manager
            .get_tb_meta_by_oid(event.rel_id() as i32)?;

        let col_values_after = self.parse_row_data(&tb_meta, event.new_tuple().tuple_data())?;

        let col_values_before = if let Some(old_tuple) = event.old_tuple() {
            self.parse_row_data(&tb_meta, old_tuple.tuple_data())?
        } else if let Some(key_tuple) = event.key_tuple() {
            self.parse_row_data(&tb_meta, key_tuple.tuple_data())?
        } else if !tb_meta.where_cols.is_empty() {
            let mut col_values_tmp = HashMap::new();
            for col in tb_meta.where_cols.iter() {
                col_values_tmp.insert(col.to_string(), col_values_after.get(col).unwrap().clone());
            }
            col_values_tmp
        } else {
            HashMap::new()
        };

        let row_data = RowData {
            db: tb_meta.schema,
            tb: tb_meta.tb,
            row_type: RowType::Update,
            before: Some(col_values_before),
            after: Some(col_values_after),
            position: "".to_string(),
        };
        self.push_row_to_buf(row_data).await
    }

    async fn decode_delete(&mut self, event: &DeleteBody) -> Result<(), Error> {
        let tb_meta = self
            .meta_manager
            .get_tb_meta_by_oid(event.rel_id() as i32)?;

        let col_values = if let Some(old_tuple) = event.old_tuple() {
            self.parse_row_data(&tb_meta, old_tuple.tuple_data())?
        } else if let Some(key_tuple) = event.key_tuple() {
            self.parse_row_data(&tb_meta, key_tuple.tuple_data())?
        } else {
            HashMap::new()
        };

        let row_data = RowData {
            db: tb_meta.schema,
            tb: tb_meta.tb,
            row_type: RowType::Delete,
            before: Some(col_values),
            after: None,
            position: "".to_string(),
        };
        self.push_row_to_buf(row_data).await
    }

    fn parse_row_data(
        &mut self,
        tb_meta: &PgTbMeta,
        tuple_data: &[TupleData],
    ) -> Result<HashMap<String, ColValue>, Error> {
        let mut col_values: HashMap<String, ColValue> = HashMap::new();
        for i in 0..tuple_data.len() {
            let tuple_data = &tuple_data[i];
            let col = &tb_meta.cols[i];
            let col_type = tb_meta.col_type_map.get(col).unwrap();

            match tuple_data {
                TupleData::Null => {
                    col_values.insert(col.to_string(), ColValue::None);
                }

                TupleData::Text(value) => {
                    let col_value =
                        PgColValueConvertor::from_wal(&col_type, &value, &mut self.meta_manager)?;
                    col_values.insert(col.to_string(), col_value);
                }

                TupleData::UnchangedToast => {
                    return Err(Error::Unexpected {
                        error: "unexpected UnchangedToast value received".to_string(),
                    })
                }
            }
        }
        Ok(col_values)
    }

    async fn push_row_to_buf(&mut self, row_data: RowData) -> Result<(), Error> {
        if self
            .filter
            .filter(&row_data.db, &row_data.tb, &row_data.row_type)
        {
            return Ok(());
        }

        while self.buffer.is_full() {
            TaskUtil::sleep_millis(1).await;
        }
        let _ = self.buffer.push(row_data);
        Ok(())
    }
}
