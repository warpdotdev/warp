use chrono::{DateTime, NaiveDateTime, Utc};
use diesel::{Connection, QueryDsl, RunQueryDsl, SqliteConnection, result::Error};
use persistence::{model::NewCloudObjectsRefresh, schema};

pub fn record_time_of_next_refresh(
    conn: &mut SqliteConnection,
    timestamp: DateTime<Utc>,
) -> Result<(), Error> {
    use schema::cloud_objects_refreshes::dsl::*;
    let refresh = NewCloudObjectsRefresh {
        time_of_next_refresh: timestamp.naive_utc(),
    };
    conn.transaction::<(), Error, _>(|conn| {
        diesel::delete(cloud_objects_refreshes).execute(conn)?;
        diesel::insert_into(cloud_objects_refreshes)
            .values(refresh)
            .execute(conn)?;
        Ok(())
    })
}

pub fn read_time_of_next_force_object_refresh(
    conn: &mut SqliteConnection,
) -> Result<Option<DateTime<Utc>>, Error> {
    use schema::cloud_objects_refreshes::dsl::*;
    // Find the smallest refresh timestamp to pass into CloudModel.
    Ok(cloud_objects_refreshes
        .select(time_of_next_refresh)
        .load::<NaiveDateTime>(conn)?
        .into_iter()
        .map(|refresh| refresh.and_utc())
        .min())
}
