use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use dt_common::{
    config::{
        config_enums::DbType, extractor_config::ExtractorConfig, filter_config::FilterConfig,
        router_config::RouterConfig, sinker_config::SinkerConfig,
    },
    utils::rdb_filter::RdbFilter,
};
use futures::{Stream, TryStreamExt};
use sqlx::{
    mysql::{MySqlPoolOptions, MySqlRow},
    query, MySql, Pool, Row,
};

use crate::{
    error::Error,
    fetcher::traits::Fetcher,
    meta::database_mode::{Constraint, Database, Schema, Table},
};

pub struct MysqlFetcher {
    pub pool: Option<Pool<MySql>>,
    pub source_config: ExtractorConfig,
    pub filter_config: FilterConfig,
    pub sinker_config: SinkerConfig,
    pub router_config: RouterConfig,
    pub is_source: bool,
    pub db_type_option: Option<DbType>,
    pub filter: RdbFilter,
}

#[async_trait]
impl Fetcher for MysqlFetcher {
    async fn build_connection(&mut self) -> Result<(), Error> {
        let mut connection_url = String::from("");

        if self.is_source {
            if let ExtractorConfig::MysqlBasic { url, .. } = &self.source_config {
                connection_url = String::from(url);
                self.db_type_option = Some(DbType::Mysql);
            }
        } else if let SinkerConfig::MysqlBasic { url, .. } = &self.sinker_config {
            connection_url = String::from(url);
            self.db_type_option = Some(DbType::Mysql);
        }
        if !connection_url.is_empty() {
            let db_pool_result = MySqlPoolOptions::new()
                .max_connections(8)
                .acquire_timeout(Duration::from_secs(5))
                .connect(connection_url.as_str())
                .await;
            match db_pool_result {
                Ok(pool) => self.pool = Option::Some(pool),
                Err(error) => return Err(Error::from(error)),
            }
        }
        Ok(())
    }

    async fn fetch_version(&mut self) -> Result<String, Error> {
        let sql = "select version()".to_string();
        let mut version: String = String::from("");

        let result = self.fetch_all(sql, "mysql query database version").await;
        match result {
            Ok(rows) => {
                if !rows.is_empty() {
                    let version_str: String = rows.get(0).unwrap().get("version()");
                    version = version_str;
                }
            }
            Err(e) => return Err(e),
        }

        Ok(version)
    }

    async fn fetch_configuration(
        &mut self,
        config_keys: Vec<String>,
    ) -> Result<HashMap<String, String>, Error> {
        if config_keys.is_empty() {
            return Ok(HashMap::new());
        }
        let sql = format!(
            "show variables where variable_name in ({})",
            config_keys
                .iter()
                .map(|c| format!("'{}'", c))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut result_map: HashMap<String, String> = config_keys
            .iter()
            .map(|c| (c.clone(), String::from("")))
            .collect();
        let result = self.fetch_all(sql, "mysql query config settings").await;
        match result {
            Ok(rows) => {
                for row in rows {
                    let (variable_name, value): (String, String) =
                        (row.get("Variable_name"), row.get("Value"));
                    if result_map.contains_key(variable_name.as_str()) {
                        result_map.insert(variable_name, value);
                    }
                }
            }
            Err(e) => return Err(e),
        }

        Ok(result_map)
    }

    async fn fetch_databases(&mut self) -> Result<Vec<Database>, Error> {
        let mut results: Vec<Database> = vec![];
        let query_db = "select schema_name from information_schema.SCHEMATA";

        let rows_result = self.fetch_row(query_db, "mysql query dbs sql:");
        match rows_result {
            Ok(mut rows) => {
                while let Some(row) = rows.try_next().await.unwrap() {
                    let schema_name: String = row.get("SCHEMA_NAME");
                    if !self.filter.filter_db(&schema_name) {
                        results.push(Database {
                            database_name: schema_name,
                        })
                    }
                }
            }
            Err(e) => return Err(e),
        }

        Ok(results)
    }

    async fn fetch_schemas(&mut self) -> Result<Vec<Schema>, Error> {
        Ok(vec![])
    }

    async fn fetch_tables(&mut self) -> Result<Vec<Table>, Error> {
        let mut results: Vec<Table> = vec![];
        let query_tb = "select t.table_schema,t.table_name from information_schema.tables t";

        let rows_result = self.fetch_row(query_tb, "mysql query tables sql:");
        match rows_result {
            Ok(mut rows) => {
                while let Some(row) = rows.try_next().await.unwrap() {
                    let (db, table): (String, String) =
                        (row.get("TABLE_SCHEMA"), row.get("TABLE_NAME"));
                    if !self.filter.filter_tb(&db, &table) {
                        results.push(Table {
                            database_name: db,
                            schema_name: String::from(""),
                            table_name: table,
                        })
                    }
                }
            }
            Err(e) => return Err(e),
        }

        Ok(results)
    }

    async fn fetch_constraints(&mut self) -> Result<Vec<Constraint>, Error> {
        let mut results: Vec<Constraint> = vec![];
        let query_constaint = "select table_schema,table_name,constraint_name, constraint_type from information_schema.table_constraints";

        let rows_result = self.fetch_row(query_constaint, "mysql query constraints sql:");
        match rows_result {
            Ok(mut rows) => {
                while let Some(row) = rows.try_next().await.unwrap() {
                    let (db, table, constraint_name, constraint_type): (
                        String,
                        String,
                        String,
                        String,
                    ) = (
                        row.get("TABLE_SCHEMA"),
                        row.get("TABLE_NAME"),
                        row.get("CONSTRAINT_NAME"),
                        row.get("CONSTRAINT_TYPE"),
                    );
                    if !self.filter.filter_tb(&db, &table) {
                        results.push(Constraint {
                            database_name: db,
                            schema_name: String::from(""),
                            table_name: table,
                            column_name: String::from(""),
                            constraint_name,
                            constraint_type,
                        })
                    }
                }
            }
            Err(e) => return Err(e),
        }

        Ok(results)
    }
}

impl MysqlFetcher {
    async fn fetch_all(&self, sql: String, mut sql_msg: &str) -> Result<Vec<MySqlRow>, Error> {
        let mysql_pool = match &self.pool {
            Some(pool) => pool,
            None => return Err(Error::from(sqlx::Error::PoolClosed)),
        };

        sql_msg = if sql_msg.is_empty() { "sql" } else { sql_msg };
        println!("{}: {}", sql_msg, sql);

        let rows_result = query(&sql).fetch_all(mysql_pool).await;
        match rows_result {
            Ok(rows) => Ok(rows),
            Err(e) => Err(Error::from(e)),
        }
    }

    fn fetch_row<'a>(
        &self,
        sql: &'a str,
        mut sql_msg: &str,
    ) -> Result<impl Stream<Item = Result<MySqlRow, sqlx::Error>> + 'a, Error> {
        match &self.pool {
            Some(pool) => {
                sql_msg = if sql_msg.is_empty() { "sql" } else { sql_msg };
                println!("{}: {}", sql_msg, sql);
                Ok(query(sql).fetch(pool))
            }
            None => Err(Error::from(sqlx::Error::PoolClosed)),
        }
    }
}