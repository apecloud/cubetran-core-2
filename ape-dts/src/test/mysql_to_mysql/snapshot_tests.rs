#[cfg(test)]
mod test {
    use crate::test::test_runner::TestRunner;
    use serial_test::serial;
    use tokio::runtime::Runtime;

    #[test]
    #[serial]
    fn snapshot_basic_test() {
        match project_root::get_project_root() {
            Ok(p) => println!("Current project root is {:?}", p),
            Err(e) => println!("Error obtaining project root {:?}", e),
        };

        let rt = Runtime::new().unwrap();
        let runner = rt
            .block_on(TestRunner::new("mysql_to_mysql/snapshot_basic_test"))
            .unwrap();
        rt.block_on(runner.run_snapshot_test(false)).unwrap();
    }

    #[test]
    #[serial]
    fn snapshot_on_duplicate_test() {
        let rt = Runtime::new().unwrap();
        let runner = rt
            .block_on(TestRunner::new("mysql_to_mysql/snapshot_on_duplicate_test"))
            .unwrap();
        rt.block_on(runner.run_snapshot_test(false)).unwrap();
    }
}