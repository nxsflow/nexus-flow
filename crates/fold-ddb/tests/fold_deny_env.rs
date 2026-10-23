//! `DynamoDbDenyList::from_env` reads `NXS_DENY_TABLE` (6j6v.dbmd): the one test that sets the
//! process's environment, alone in its binary so no other test can see the variable change.
#![cfg(feature = "dynamodb")]

#[path = "support/ddb.rs"]
mod ddb;

use nxs_fold_ddb::ddb::{create_deny_table, create_table, DenyTableError, DynamoDbDenyList};
use nxs_fold_ddb::deny::{AccountHash, DenyList, DENY_TABLE_ENV};

#[test]
fn from_env_refuses_every_wrong_deny_table_and_reads_the_right_one() {
    let Some(endpoint) = ddb::endpoint("from_env_refuses_every_wrong_deny_table") else {
        return;
    };
    let client = ddb::client(&endpoint);
    let (fold, deny) = (ddb::unique("nxf_env_fold"), ddb::unique("nxf_env_deny"));
    ddb::rt().block_on(async {
        create_table(&client, &fold).send().await.unwrap();
        create_deny_table(&client, &deny).send().await.unwrap();
    });
    ddb::wait_active(&client, &fold);
    ddb::wait_active(&client, &deny);
    let open = || ddb::rt().block_on(DynamoDbDenyList::from_env(client.clone()));

    std::env::remove_var(DENY_TABLE_ENV);
    assert_eq!(open().unwrap_err(), DenyTableError::Unset);
    std::env::set_var(DENY_TABLE_ENV, "");
    assert_eq!(open().unwrap_err(), DenyTableError::Unset);
    std::env::set_var(DENY_TABLE_ENV, "nxf_env_no_such_table");
    assert_eq!(open().unwrap_err(), DenyTableError::Missing);
    assert!(!ddb::table_names(&client).contains(&"nxf_env_no_such_table".to_string()));
    std::env::set_var(DENY_TABLE_ENV, &fold);
    assert!(matches!(
        open().unwrap_err(),
        DenyTableError::NotADenyTable(_)
    ));

    std::env::set_var(DENY_TABLE_ENV, &deny);
    let list = open().unwrap();
    assert_eq!(list.table().map(|t| t.name.as_str()), Some(deny.as_str()));
    let h = AccountHash::parse("00ff00ff00ff00ff00ff00ff00ff00ff").unwrap();
    assert!(!ddb::rt().block_on(list.is_denied(&h)).unwrap());
    ddb::put_raw(
        &client,
        &deny,
        &[
            ("h", h.as_str()),
            ("v", "#N:1"),
            ("since", "2026-10-23T10:00:00Z"),
        ],
    );
    assert!(ddb::rt().block_on(list.is_denied(&h)).unwrap());
    std::env::remove_var(DENY_TABLE_ENV);
    ddb::delete_table(&client, &fold);
    ddb::delete_table(&client, &deny);
}
