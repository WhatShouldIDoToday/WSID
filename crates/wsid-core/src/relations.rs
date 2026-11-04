use std::collections::{BTreeSet, HashSet, VecDeque};
use std::str::FromStr;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Database;
use crate::error::{Result, WsidError};
use crate::id::normalize_task_id;
use crate::models::{RelationInputKind, RelationType, Task, TaskRelation};
use crate::tasks::{format_utc, get_task_conn, parse_utc};

/// Links `source_id` and `target_id` using any [`RelationInputKind`], normalizing
/// inverse/symmetric relations and enforcing DAG cycle prevention for `blocks` and `parent_of`.
pub fn link_tasks(db: &mut Database, raw_source_id: &str, input_kind: RelationInputKind, raw_target_id: &str) -> Result<TaskRelation> {
    let tx = db.conn.transaction()?;
    let rel = link_tasks_in_tx(&tx, raw_source_id, input_kind, raw_target_id)?;
    tx.commit()?;
    Ok(rel)
}

pub(crate) fn link_tasks_in_tx(
    conn: &Connection,
    raw_source_id: &str,
    input_kind: RelationInputKind,
    raw_target_id: &str,
) -> Result<TaskRelation> {
    let source_norm = normalize_task_id(raw_source_id)?;
    let target_norm = normalize_task_id(raw_target_id)?;

    if source_norm == target_norm {
        return Err(WsidError::Validation(format!("cannot link task {source_norm} to itself")));
    }

    // Ensure both tasks exist
    ensure_task_exists(conn, &source_norm)?;
    ensure_task_exists(conn, &target_norm)?;

    let (canon_src, canon_dst, rel_type) = input_kind.canonicalize(&source_norm, &target_norm);

    if rel_type.is_directed_acyclic() && would_create_cycle_conn(conn, canon_src, canon_dst, rel_type)? {
        return Err(WsidError::CycleDetected {
            source_id: canon_src.to_string(),
            target_id: canon_dst.to_string(),
            relation: rel_type.as_str().to_string(),
        });
    }

    let now_dt = Utc::now();
    let now_str = format_utc(now_dt);

    conn.execute(
        "INSERT OR IGNORE INTO task_relations (source_id, target_id, relation_type, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![canon_src, canon_dst, rel_type.as_str(), now_str],
    )?;

    let created_str: String = conn.query_row(
        "SELECT created_at FROM task_relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = ?3",
        params![canon_src, canon_dst, rel_type.as_str()],
        |r| r.get(0),
    )?;

    Ok(TaskRelation {
        source_id: canon_src.to_string(),
        target_id: canon_dst.to_string(),
        relation_type: rel_type,
        created_at: parse_utc(&created_str),
    })
}

/// Removes a relation between `raw_source_id` and `raw_target_id` after canonicalizing `input_kind`.
pub fn unlink_tasks(db: &mut Database, raw_source_id: &str, input_kind: RelationInputKind, raw_target_id: &str) -> Result<bool> {
    unlink_tasks_conn(&db.conn, raw_source_id, input_kind, raw_target_id)
}

fn unlink_tasks_conn(conn: &Connection, raw_source_id: &str, input_kind: RelationInputKind, raw_target_id: &str) -> Result<bool> {
    let source_norm = normalize_task_id(raw_source_id)?;
    let target_norm = normalize_task_id(raw_target_id)?;
    let (canon_src, canon_dst, rel_type) = input_kind.canonicalize(&source_norm, &target_norm);

    let affected = if rel_type.is_symmetric() {
        conn.execute(
            "DELETE FROM task_relations
             WHERE relation_type = ?1
               AND ((source_id = ?2 AND target_id = ?3) OR (source_id = ?3 AND target_id = ?2))",
            params![rel_type.as_str(), canon_src, canon_dst],
        )?
    } else {
        conn.execute(
            "DELETE FROM task_relations
             WHERE source_id = ?1 AND target_id = ?2 AND relation_type = ?3",
            params![canon_src, canon_dst, rel_type.as_str()],
        )?
    };

    Ok(affected > 0)
}

/// Replaces the relation between `raw_source_id` and `raw_target_id` from `old_kind` to `new_kind`.
///
/// Both steps run in one transaction so inverse rewrites (`blocks` <-> `blocked_by`,
/// `parent_task_of` <-> `sub_task_of`) on the same task pair do not trip the DAG cycle guard,
/// and a genuine cycle elsewhere rolls back without losing the original relation.
pub fn update_task_relation(
    db: &mut Database,
    raw_source_id: &str,
    old_kind: RelationInputKind,
    new_kind: RelationInputKind,
    raw_target_id: &str,
) -> Result<TaskRelation> {
    if old_kind == new_kind {
        return link_tasks(db, raw_source_id, new_kind, raw_target_id);
    }

    let tx = db.conn.transaction()?;
    unlink_tasks_conn(&tx, raw_source_id, old_kind, raw_target_id)?;
    let rel = link_tasks_in_tx(&tx, raw_source_id, new_kind, raw_target_id)?;
    tx.commit()?;
    Ok(rel)
}

/// Lists all canonical relations where `raw_task_id` is either `source_id` or `target_id`.
pub fn list_task_relations(db: &Database, raw_task_id: &str) -> Result<Vec<TaskRelation>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let mut stmt = db.conn.prepare_cached(
        "SELECT source_id, target_id, relation_type, created_at
         FROM task_relations
         WHERE source_id = ?1 OR target_id = ?1
         ORDER BY created_at ASC, source_id ASC, target_id ASC",
    )?;

    let rows = stmt.query_map([&task_id], |row| {
        let rel_str: String = row.get(2)?;
        let created_str: String = row.get(3)?;
        Ok(TaskRelation {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            relation_type: RelationType::from_str(&rel_str).unwrap_or(RelationType::Related),
            created_at: parse_utc(&created_str),
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Checks whether adding the directed edge `source_id -> target_id` of `rel_type` would introduce a cycle.
pub fn would_create_cycle(db: &Database, source_id: &str, target_id: &str, rel_type: RelationType) -> Result<bool> {
    would_create_cycle_conn(&db.conn, source_id, target_id, rel_type)
}

pub(crate) fn would_create_cycle_conn(conn: &Connection, source_id: &str, target_id: &str, rel_type: RelationType) -> Result<bool> {
    if source_id == target_id {
        return Ok(true);
    }
    if !rel_type.is_directed_acyclic() {
        return Ok(false);
    }

    // Adding `source_id -> target_id` creates a cycle iff `source_id` is already reachable from `target_id`.
    let mut visited = HashSet::new();
    let mut stack = vec![target_id.to_string()];

    let mut stmt = conn.prepare_cached(
        "SELECT target_id
         FROM task_relations
         WHERE source_id = ?1 AND relation_type = ?2",
    )?;

    while let Some(curr) = stack.pop() {
        if curr == source_id {
            return Ok(true);
        }
        if !visited.insert(curr.clone()) {
            continue;
        }
        let rows = stmt.query_map(params![curr, rel_type.as_str()], |row| row.get::<_, String>(0))?;
        for next in rows {
            let next_id = next?;
            if next_id == source_id {
                return Ok(true);
            }
            if !visited.contains(&next_id) {
                stack.push(next_id);
            }
        }
    }

    Ok(false)
}

/// Returns direct blocker tasks (`B` where `B blocks task_id`).
pub fn get_direct_blockers(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let mut stmt = db.conn.prepare_cached(
        "SELECT source_id
         FROM task_relations
         WHERE target_id = ?1 AND relation_type = 'blocks'
         ORDER BY created_at ASC, source_id ASC",
    )?;
    let ids: Vec<String> = stmt
        .query_map([&task_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns all transitive blocker tasks (`B` where `B ->+ blocks -> task_id`).
pub fn get_transitive_blockers(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let ids = traverse_graph(&db.conn, &task_id, RelationType::Blocks, GraphDirection::Incoming)?;
    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns direct downstream tasks that `task_id` blocks (`D` where `task_id blocks D`).
pub fn get_direct_blocked_tasks(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let mut stmt = db.conn.prepare_cached(
        "SELECT target_id
         FROM task_relations
         WHERE source_id = ?1 AND relation_type = 'blocks'
         ORDER BY created_at ASC, target_id ASC",
    )?;
    let ids: Vec<String> = stmt
        .query_map([&task_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns all transitive downstream tasks that `task_id` blocks (`D` where `task_id ->+ blocks -> D`).
pub fn get_transitive_blocked_tasks(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let ids = traverse_graph(&db.conn, &task_id, RelationType::Blocks, GraphDirection::Outgoing)?;
    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns direct subtasks (`C` where `task_id parent_of C`).
pub fn get_direct_subtasks(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let mut stmt = db.conn.prepare_cached(
        "SELECT target_id
         FROM task_relations
         WHERE source_id = ?1 AND relation_type = 'parent_of'
         ORDER BY created_at ASC, target_id ASC",
    )?;
    let ids: Vec<String> = stmt
        .query_map([&task_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns all transitive subtasks (`C` where `task_id ->+ parent_of -> C`).
pub fn get_transitive_subtasks(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let ids = traverse_graph(&db.conn, &task_id, RelationType::ParentOf, GraphDirection::Outgoing)?;
    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns direct parent tasks (`P` where `P parent_of task_id`).
pub fn get_direct_parents(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let mut stmt = db.conn.prepare_cached(
        "SELECT source_id
         FROM task_relations
         WHERE target_id = ?1 AND relation_type = 'parent_of'
         ORDER BY created_at ASC, source_id ASC",
    )?;
    let ids: Vec<String> = stmt
        .query_map([&task_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns the ancestor/parent chain in breadth-first order (`P` where `P ->+ parent_of -> task_id`).
pub fn get_parent_chain(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let ids = traverse_graph(&db.conn, &task_id, RelationType::ParentOf, GraphDirection::Incoming)?;
    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns tasks linked via `parallel` to `raw_task_id`.
pub fn get_parallel_peers(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let ids = get_symmetric_peers(&db.conn, &task_id, RelationType::Parallel)?;
    load_tasks_by_ids(&db.conn, &ids)
}

/// Returns tasks linked via `related` to `raw_task_id`.
pub fn get_related_tasks(db: &Database, raw_task_id: &str) -> Result<Vec<Task>> {
    let task_id = normalize_task_id(raw_task_id)?;
    ensure_task_exists(&db.conn, &task_id)?;

    let ids = get_symmetric_peers(&db.conn, &task_id, RelationType::Related)?;
    load_tasks_by_ids(&db.conn, &ids)
}

#[derive(Clone, Copy)]
enum GraphDirection {
    Outgoing,
    Incoming,
}

fn traverse_graph(conn: &Connection, start_id: &str, rel_type: RelationType, direction: GraphDirection) -> Result<Vec<String>> {
    let sql = match direction {
        GraphDirection::Outgoing => {
            "SELECT target_id FROM task_relations WHERE source_id = ?1 AND relation_type = ?2 ORDER BY created_at ASC, target_id ASC"
        }
        GraphDirection::Incoming => {
            "SELECT source_id FROM task_relations WHERE target_id = ?1 AND relation_type = ?2 ORDER BY created_at ASC, source_id ASC"
        }
    };

    let mut stmt = conn.prepare_cached(sql)?;
    let mut visited = HashSet::new();
    visited.insert(start_id.to_string());

    let mut queue = VecDeque::new();
    queue.push_back(start_id.to_string());

    let mut ordered = Vec::new();
    while let Some(curr) = queue.pop_front() {
        let rows = stmt.query_map(params![curr, rel_type.as_str()], |r| r.get::<_, String>(0))?;
        for next_res in rows {
            let next_id = next_res?;
            if visited.insert(next_id.clone()) {
                ordered.push(next_id.clone());
                queue.push_back(next_id);
            }
        }
    }

    Ok(ordered)
}

fn get_symmetric_peers(conn: &Connection, task_id: &str, rel_type: RelationType) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT CASE WHEN source_id = ?1 THEN target_id ELSE source_id END AS peer_id
         FROM task_relations
         WHERE (source_id = ?1 OR target_id = ?1) AND relation_type = ?2
         ORDER BY created_at ASC, peer_id ASC",
    )?;
    let rows = stmt.query_map(params![task_id, rel_type.as_str()], |r| r.get::<_, String>(0))?;
    let mut seen = BTreeSet::new();
    let mut peers = Vec::new();
    for r in rows {
        let id = r?;
        if seen.insert(id.clone()) {
            peers.push(id);
        }
    }
    Ok(peers)
}

pub(crate) fn ensure_task_exists(conn: &Connection, normalized_id: &str) -> Result<()> {
    let exists: Option<i64> = conn
        .query_row("SELECT 1 FROM tasks WHERE id = ?1", [normalized_id], |r| r.get(0))
        .optional()?;
    if exists.is_none() {
        return Err(WsidError::TaskNotFound(normalized_id.to_string()));
    }
    Ok(())
}

fn load_tasks_by_ids(conn: &Connection, ids: &[String]) -> Result<Vec<Task>> {
    let mut tasks = Vec::with_capacity(ids.len());
    for id in ids {
        tasks.push(get_task_conn(conn, id)?);
    }
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory_db;
    use crate::models::CreateTaskInput;
    use crate::tasks::create_task;

    fn make_task(db: &mut Database, title: &str) -> Task {
        create_task(db, CreateTaskInput { title: title.to_string(), ..Default::default() }, "alice").unwrap()
    }

    #[test]
    fn maps_inverse_and_symmetric_relations_to_canonical_edges() {
        let mut db = open_in_memory_db().unwrap();
        let a = make_task(&mut db, "Task A");
        let b = make_task(&mut db, "Task B");
        let c = make_task(&mut db, "Task C");

        // `A blocked_by B` stores `B blocks A`
        let r1 = link_tasks(&mut db, &a.id, RelationInputKind::BlockedBy, &b.id).unwrap();
        assert_eq!(r1.source_id, b.id);
        assert_eq!(r1.target_id, a.id);
        assert_eq!(r1.relation_type, RelationType::Blocks);

        // `C sub_task_of A` stores `A parent_of C`
        let r2 = link_tasks(&mut db, &c.id, RelationInputKind::SubTaskOf, &a.id).unwrap();
        assert_eq!(r2.source_id, a.id);
        assert_eq!(r2.target_id, c.id);
        assert_eq!(r2.relation_type, RelationType::ParentOf);

        // Symmetric `parallel` deduplicates regardless of direction
        let r3 = link_tasks(&mut db, &b.id, RelationInputKind::Parallel, &c.id).unwrap();
        let r4 = link_tasks(&mut db, &c.id, RelationInputKind::Parallel, &b.id).unwrap();
        assert_eq!(r3.source_id, r4.source_id);
        assert_eq!(r3.target_id, r4.target_id);

        let peers_b = get_parallel_peers(&db, &b.id).unwrap();
        assert_eq!(peers_b.len(), 1);
        assert_eq!(peers_b[0].id, c.id);

        // Unlinking via inverse `BlockedBy` removes the canonical `Blocks` edge
        assert!(unlink_tasks(&mut db, &a.id, RelationInputKind::BlockedBy, &b.id).unwrap());
        assert!(get_direct_blockers(&db, &a.id).unwrap().is_empty());
    }

    #[test]
    fn rejects_self_links_and_multi_hop_dag_cycles() {
        let mut db = open_in_memory_db().unwrap();
        let a = make_task(&mut db, "Task A");
        let b = make_task(&mut db, "Task B");
        let c = make_task(&mut db, "Task C");

        // Self link rejected
        let self_err = link_tasks(&mut db, &a.id, RelationInputKind::Blocks, &a.id).unwrap_err();
        assert!(matches!(self_err, WsidError::Validation(_)));

        // A blocks B, B blocks C
        link_tasks(&mut db, &a.id, RelationInputKind::Blocks, &b.id).unwrap();
        link_tasks(&mut db, &b.id, RelationInputKind::Blocks, &c.id).unwrap();

        // C blocks A must fail with CycleDetected
        let cycle_err = link_tasks(&mut db, &c.id, RelationInputKind::Blocks, &a.id).unwrap_err();
        assert!(matches!(cycle_err, WsidError::CycleDetected { .. }));

        // Also test via inverse: A blocked_by C (which means C blocks A)
        let inv_cycle_err = link_tasks(&mut db, &a.id, RelationInputKind::BlockedBy, &c.id).unwrap_err();
        assert!(matches!(inv_cycle_err, WsidError::CycleDetected { .. }));

        // Parent-child cycle detection: A parent_of B, B parent_of C -> C parent_of A rejected
        link_tasks(&mut db, &a.id, RelationInputKind::ParentTaskOf, &b.id).unwrap();
        link_tasks(&mut db, &b.id, RelationInputKind::ParentTaskOf, &c.id).unwrap();
        let parent_cycle = link_tasks(&mut db, &a.id, RelationInputKind::SubTaskOf, &c.id).unwrap_err();
        assert!(matches!(parent_cycle, WsidError::CycleDetected { .. }));

        // Verify transitive traversal helpers
        let transitive_blockers_c = get_transitive_blockers(&db, &c.id).unwrap();
        let blocker_ids: Vec<&str> = transitive_blockers_c
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(blocker_ids, vec![b.id.as_str(), a.id.as_str()]);

        let parent_chain_c = get_parent_chain(&db, &c.id).unwrap();
        let parent_ids: Vec<&str> = parent_chain_c.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(parent_ids, vec![b.id.as_str(), a.id.as_str()]);
    }

    #[test]
    fn updates_relation_type_atomically() {
        let mut db = open_in_memory_db().unwrap();
        let a = make_task(&mut db, "Task A");
        let b = make_task(&mut db, "Task B");
        let c = make_task(&mut db, "Task C");

        // Flip the orientation of an existing edge: A blocks B -> A blocked_by B (B blocks A).
        link_tasks(&mut db, &a.id, RelationInputKind::Blocks, &b.id).unwrap();
        let flipped = update_task_relation(&mut db, &a.id, RelationInputKind::Blocks, RelationInputKind::BlockedBy, &b.id).unwrap();
        assert_eq!(flipped.relation_type, RelationType::Blocks);
        assert_eq!(flipped.source_id, b.id);
        assert_eq!(flipped.target_id, a.id);
        assert!(get_direct_blockers(&db, &b.id).unwrap().is_empty());
        let blockers_a = get_direct_blockers(&db, &a.id).unwrap();
        assert_eq!(blockers_a.len(), 1);
        assert_eq!(blockers_a[0].id, b.id);

        // Parent <-> child rewrite on the same pair is allowed (no self-cycle false positive).
        link_tasks(&mut db, &a.id, RelationInputKind::ParentTaskOf, &c.id).unwrap();
        let reparented =
            update_task_relation(&mut db, &a.id, RelationInputKind::ParentTaskOf, RelationInputKind::SubTaskOf, &c.id).unwrap();
        assert_eq!(reparented.source_id, c.id);
        assert_eq!(reparented.target_id, a.id);
        assert_eq!(get_direct_subtasks(&db, &a.id).unwrap().len(), 0);
        assert_eq!(get_direct_parents(&db, &a.id).unwrap()[0].id, c.id);

        // A genuine cycle rolls back and keeps the original relation intact.
        // After the flip B blocks A; add A blocks C so B -> A -> C is a chain.
        link_tasks(&mut db, &a.id, RelationInputKind::Blocks, &c.id).unwrap();
        link_tasks(&mut db, &c.id, RelationInputKind::Related, &b.id).unwrap();
        let cycle_err = update_task_relation(&mut db, &c.id, RelationInputKind::Related, RelationInputKind::Blocks, &b.id).unwrap_err();
        assert!(matches!(cycle_err, WsidError::CycleDetected { .. }));
        assert!(
            get_related_tasks(&db, &c.id)
                .unwrap()
                .iter()
                .any(|t| t.id == b.id)
        );
        assert!(
            !get_direct_blocked_tasks(&db, &c.id)
                .unwrap()
                .iter()
                .any(|t| t.id == b.id)
        );
        assert_eq!(get_direct_blockers(&db, &a.id).unwrap()[0].id, b.id);
    }
}
