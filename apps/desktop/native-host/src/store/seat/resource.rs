//! E.2 host resource observation. The operating system is the source of the
//! input, and the Owner's persisted project limit remains a separate row.

use super::*;

pub(super) const HOST_RESOURCES: &str = "CREATE TABLE gogoke_v37_seat_host_resources(singleton INTEGER PRIMARY KEY CHECK(singleton=1),source TEXT NOT NULL CHECK(source='STD_AVAILABLE_PARALLELISM'),observed_parallelism INTEGER NOT NULL CHECK(observed_parallelism>0),machine_limit INTEGER NOT NULL CHECK(machine_limit>0),revision INTEGER NOT NULL CHECK(revision>0)) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostParallelFact {
    pub(crate) observed_parallelism: i64,
    pub(crate) machine_limit: i64,
    pub(crate) revision: i64,
}

fn observed_parallelism() -> Result<i64, SeatError> {
    let count = std::thread::available_parallelism().map_err(|_|SeatError::Denied)?.get();
    i64::try_from(count).map_err(|_|SeatError::Denied)
}

/// H calls this inside its existing BEGIN IMMEDIATE admission transaction.
/// No caller supplies a capacity or changes the Owner's project-cap row.
pub(crate) fn refresh_host_parallel_fact_in_transaction(
    db: &VerifiedDatabaseConnection<'_>,
) -> Result<HostParallelFact, SeatError> {
    let observed=observed_parallelism()?;
    if observed<=0 { return Err(SeatError::Denied); }
    let update=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_seat_host_resources(singleton,source,observed_parallelism,machine_limit,revision) VALUES(1,'STD_AVAILABLE_PARALLELISM',?1,?1,1) ON CONFLICT(singleton) DO UPDATE SET observed_parallelism=excluded.observed_parallelism,machine_limit=excluded.machine_limit,revision=revision+1 WHERE source='STD_AVAILABLE_PARALLELISM' AND observed_parallelism!=excluded.observed_parallelism")?;
    update.bind_i64(1,observed)?; update.step_done()?;
    read_host_parallel_fact(db)
}

pub(crate) fn read_host_parallel_fact(
    db: &VerifiedDatabaseConnection<'_>,
) -> Result<HostParallelFact, SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT source,observed_parallelism,machine_limit,revision FROM main.gogoke_v37_seat_host_resources WHERE singleton=1")?;
    if !q.step_row()? { return Err(SeatError::Denied); }
    if q.column_text(0)?!="STD_AVAILABLE_PARALLELISM" { return Err(SeatError::SchemaDrift); }
    let parse=|index|q.column_text(index)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift);
    let fact=HostParallelFact {observed_parallelism:parse(1)?,machine_limit:parse(2)?,revision:parse(3)?};
    if fact.observed_parallelism<=0 || fact.machine_limit!=fact.observed_parallelism ||
        fact.revision<=0 || q.step_row()? { return Err(SeatError::SchemaDrift); }
    Ok(fact)
}

pub(crate) fn read_effective_project_parallel_cap(
    db: &VerifiedDatabaseConnection<'_>, domain_id: &str,
) -> Result<(i64,HostParallelFact), SeatError> {
    let owner=read_project_parallel_cap(db,domain_id)?;
    let machine=read_host_parallel_fact(db)?;
    Ok((owner.min(machine.machine_limit),machine))
}
