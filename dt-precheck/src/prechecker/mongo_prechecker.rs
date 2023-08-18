use async_trait::async_trait;
use dt_common::config::{config_enums::DbType, filter_config::FilterConfig};
use mongodb::bson::Bson;
use regex::Regex;

use crate::{
    config::precheck_config::PrecheckConfig,
    error::Error,
    fetcher::{mongo::mongo_fetcher::MongoFetcher, traits::Fetcher},
    meta::{check_item::CheckItem, check_result::CheckResult},
};

use super::traits::Prechecker;

const MONGO_SUPPORTED_VERSION_REGEX: &str = r"5.*|6.0.*";

pub struct MongoPrechecker {
    pub fetcher: MongoFetcher,
    pub filter_config: FilterConfig,
    pub precheck_config: PrecheckConfig,
    pub is_source: bool,
    pub db_type_option: Option<DbType>,
}

#[async_trait]
impl Prechecker for MongoPrechecker {
    async fn build_connection(&mut self) -> Result<CheckResult, Error> {
        let result = self.fetcher.build_connection().await;
        match result {
            Ok(_) => {}
            Err(e) => return Err(e),
        }

        Ok(CheckResult::build_with_err(
            CheckItem::CheckDatabaseConnection,
            self.is_source,
            self.db_type_option.clone(),
            None,
        ))
    }

    async fn check_database_version(&mut self) -> Result<CheckResult, Error> {
        let mut check_error: Option<Error> = None;

        let version = self.fetcher.fetch_version().await?;
        let reg = Regex::new(MONGO_SUPPORTED_VERSION_REGEX).unwrap();
        if !reg.is_match(version.as_str()) {
            check_error = Some(Error::PreCheckError {
                error: format!("mongo version:[{}] is invalid.", version),
            });
        }

        Ok(CheckResult::build_with_err(
            CheckItem::CheckDatabaseVersionSupported,
            self.is_source,
            self.db_type_option.clone(),
            check_error,
        ))
    }

    async fn check_permission(&mut self) -> Result<CheckResult, Error> {
        Ok(CheckResult::build(
            CheckItem::CheckAccountPermission,
            self.is_source,
        ))
    }

    async fn check_cdc_supported(&mut self) -> Result<CheckResult, Error> {
        let mut check_error: Option<Error> = None;

        if !self.is_source {
            // do nothing when the database is a target
            return Ok(CheckResult::build_with_err(
                CheckItem::CheckIfDatabaseSupportCdc,
                self.is_source,
                self.db_type_option.clone(),
                check_error,
            ));
        }

        // 1. replSet used
        // 2. the specify url is the master
        let random_db = self.fetcher.get_random_db()?;
        let rs_status = self
            .fetcher
            .execute_for_db(random_db.clone(), "hello")
            .await?;

        let (ok, primary, me): (bool, &str, &str) = (
            rs_status.get("ok").and_then(Bson::as_f64).unwrap_or(0.0) >= 1.0,
            rs_status
                .get("primary")
                .and_then(Bson::as_str)
                .unwrap_or(""),
            rs_status.get("me").and_then(Bson::as_str).unwrap_or(""),
        );

        let mut err_msg = "";
        if !ok {
            err_msg = "fetching mongodb instance status with 'db.hello()' failed.";
        } else if primary.is_empty() || me.is_empty() {
            err_msg = "mongodb is not a replicaSet architecture.";
        } else if primary != me {
            err_msg = "the mongodb instance is not a master.";
        }

        if !err_msg.is_empty() {
            check_error = Some(Error::PreCheckError {
                error: String::from(err_msg),
            });
        }

        Ok(CheckResult::build_with_err(
            CheckItem::CheckIfDatabaseSupportCdc,
            self.is_source,
            self.db_type_option.clone(),
            check_error,
        ))
    }

    async fn check_struct_existed_or_not(&mut self) -> Result<CheckResult, Error> {
        Ok(CheckResult::build_with_err(
            CheckItem::CheckIfStructExisted,
            self.is_source,
            self.db_type_option.clone(),
            None,
        ))
    }

    async fn check_table_structs(&mut self) -> Result<CheckResult, Error> {
        let mut check_error: Option<Error> = None;

        let invalid_dbs = vec!["admin", "local"];
        for db in invalid_dbs {
            if !self.fetcher.filter.filter_db(db) {
                check_error = Some(Error::PreCheckError {
                    error: String::from(
                        "database 'admin' and 'local' are not supported as source and target.",
                    ),
                });
                break;
            }
        }

        Ok(CheckResult::build_with_err(
            CheckItem::CheckIfTableStructSupported,
            self.is_source,
            self.db_type_option.clone(),
            check_error,
        ))
    }
}