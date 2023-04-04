use edgedb_derive::Queryable;
use edgedb_protocol::value::Value;
use serde::Deserialize;
use uuid::Uuid;

// The username field on Account has an exclusive constraint, plus
// giving a different name each time looks better
fn random_user_argument() -> (String,) {
    let suffix = std::iter::repeat_with(fastrand::alphanumeric)
        .take(5)
        .collect::<String>();
    (format!("User_{suffix}"),)
}

fn display_result(query: &str, res: &impl std::fmt::Debug) {
    println!("Queried: {query}\nResult: {res:?}\n");
}

// Represents the Account type in the schema, only implements Deserialize
#[derive(Debug, Deserialize)]
pub struct Account {
    pub username: String,
    pub id: Uuid,
}

// Implements Queryable on top of Deserialize so is more convenient.
// Note: Queryable requires query fields to be in the same order as the struct.
// So `select Account { id, username }` will generate a DescriptorMismatch::WrongField error
// whereas `select Account { username, id }` will not
#[derive(Debug, Deserialize, Queryable)]
pub struct QueryableAccount {
    pub username: String,
    pub id: Uuid,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // create_client() is the easiest way to create a client to access EdgeDB.
    // If there are any problems with setting up the client automatically
    // or if you need a more manual setup (e.g. reading from environment variables)
    // it can be done step by step starting with a Builder. e.g.:
    // let mut builder = edgedb_tokio::Builder::uninitialized();
    // Read from environment variables:
    // builder.read_env_vars().unwrap();
    // Or read from named instance:
    // builder.read_instance("name_of_your_instance_here").unwrap();
    // let config = builder.build().unwrap();
    // let client = edgedb_tokio::Client::new(&config);
    let client = edgedb_tokio::create_client().await?;

    // Now that the client is set up,
    // first just select a string and return it
    let query = "select {'This is a query fetching a string'}";
    let query_res: String = client.query_required_single(query, &()).await?;
    display_result(query, &query_res);
    assert_eq!(query_res, "This is a query fetching a string");

    // Selecting a tuple with two scalar types this time
    let query = "select {('Hi', 9.8)}";
    let query_res: Value = client.query_required_single(query, &()).await?;
    display_result(query, &query_res);
    assert_eq!(
        query_res,
        Value::Tuple(vec![Value::Str("Hi".to_string()), Value::Float64(9.8)])
    );
    assert_eq!(
        format!("{query_res:?}"),
        r#"Tuple([Str("Hi"), Float64(9.8)])"#
    );

    // You can pass in arguments too via a tuple
    let query = "select {(<str>$0, <int32>$1)}";
    let arguments = ("Hi there", 10);
    let query_res: Value = client.query_required_single(query, &arguments).await?;
    display_result(query, &query_res);
    assert_eq!(
        format!("{query_res:?}"),
        r#"Tuple([Str("Hi there"), Int32(10)])"#
    );

    // Arguments in queries are used as type inference for the EdgeDB compiler,
    // not to dynamically cast queries from the Rust side. So this will return an error:
    let query = "select <int32>$0";
    let argument = 9i16;
    let query_res: Result<Value, _> = client.query_required_single(query, &(argument,)).await;
    assert!(query_res.is_err());

    // Note: most scalar types have an exact match with Rust (e.g. an int32 matches a Rust i32)
    // while the internals of those that don't can be seen on the edgedb_protocol crate.
    // e.g. a BigInt can be seen here https://docs.rs/edgedb-protocol/latest/edgedb_protocol/model/struct.BigInt.html
    // and looks like this:
    //
    // pub struct BigInt {
    //     pub(crate) negative: bool,
    //     pub(crate) weight: i16,
    //     pub(crate) digits: Vec<u16>,
    // }
    //
    // and implements From for all the types you would expect.

    // Thus this query will not work:
    let query = "select <bigint>$0";
    let argument = 20;
    let query_res: Result<Value, _> = client.query_required_single(query, &(argument,)).await;
    assert!(query_res.is_err());

    // But this one will:
    let query = "select <bigint>$0";
    let bigint_arg = edgedb_protocol::model::BigInt::from(20i32);
    let query_res: Value = client.query_required_single(query, &(bigint_arg,)).await?;
    display_result(query, &query_res);
    assert_eq!(
        format!("{query_res:?}"),
        "BigInt(BigInt { negative: false, weight: 0, digits: [20] })"
    );
    // To view the rest of the implementations for scalar types, see here:
    // https://github.com/edgedb/edgedb-rust/blob/master/edgedb-protocol/src/serialization/decode/queryable/scalars.rs#L45

    // Next insert a user account. Not SELECTing anything in particular
    // So it will return a Uuid (the object's id)
    let query = "insert Account {
        username := <str>$0
        };";
    let query_res: Value = client
        .query_required_single(query, &random_user_argument())
        .await?;
    // This time we queried for a Value, which is a big enum of all the types
    // that EdgeDB supports. Just printing it out includes both the shape info and the fields
    display_result(query, &query_res);

    // We know it's a Value::Object. Let's match on the enum
    match query_res {
        // The fields property is a Vec<Option<Value>>. In this case we'll only have one:
        Value::Object { shape: _, fields } => {
            println!("Insert worked, Fields are: {fields:?}\n");
            for field in fields {
                match field {
                    Some(Value::Uuid(uuid)) => {
                        println!("Only returned one field, a Uuid: {uuid}\n")
                    }
                    _other => println!("This shouldn't happen"),
                }
            }
        }
        _other => println!("This shouldn't happen"),
    };

    // Now do the same insert as before but we'll select a shape to return instead of just the id.
    let query = "select (
        insert Account {
        username := <str>$0
      }) {
        username, 
        id
      };";
    if let Value::Object { shape: _, fields } = client
        .query_required_single(query, &random_user_argument())
        .await?
    {
        // This time we have more than one field in the fields property
        for field in fields {
            println!("Got a field: {field:?}");
        }
        println!();
    }

    // Now the same query as above, except we'll ask EdgeDB to cast it to json.
    let query = "select <json>(
        insert Account {
        username := <str>$0
      }) {
        username, 
        id
      };";

    // We know there will only be one result so use query_single_json; otherwise it will return a map of json
    let json_res = client
        .query_single_json(query, &random_user_argument())
        .await?
        .unwrap();
    println!("Json res is pretty easy:");
    display_result(query, &json_res);

    // You can turn this into a serde Value and access using square brackets:
    let as_value: serde_json::Value = serde_json::from_str(&json_res)?;
    println!(
        "Username is {},\nId is {}.\n",
        as_value["username"], as_value["id"]
    );

    // But Deserialize is much more common (and rigorous).
    // Our Account struct implements Deserialize so we can use serde_json to deserialize the result into an Account:
    let as_account: Account = serde_json::from_str(&json_res)?;
    println!("Deserialized: {as_account:?}\n");

    // But EdgeDB's Rust client has a built-in Queryable macro that lets us just query without having
    // to cast to json. Same query as before:
    let query = "select (
        insert Account {
        username := <str>$0
      }) {
        username, 
        id
      };";
    let as_queryable_account: QueryableAccount = client
        .query_required_single(query, &random_user_argument())
        .await?;
    println!("As QueryableAccount, no need for intermediate json: {as_queryable_account:?}\n");

    // And changing the order of the fields from `username, id` to `id, username` will
    // return a DescriptorMismatch::WrongField error
    let query = "select (
        insert Account {
        username := <str>$0
      }) {
        id, 
        username
      };";
    let cannot_make_into_queryable_account: Result<QueryableAccount, _> = client
        .query_required_single(query, &random_user_argument())
        .await;
    assert_eq!(
        format!("{cannot_make_into_queryable_account:?}"),
        r#"Err(Error(Inner { code: 4278386176, messages: [], error: Some(WrongField { unexpected: "id", expected: "username" }), headers: {} }))"#
    );

    Ok(())
}
