use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::error::Result;
use crate::models::{FocusPreset, OriginKind, RelationType, Task, TaskStatus};
use crate::tasks::{list_tasks, normalize_tag, normalize_user};

/// Default workday length in minutes (8 hours = 480 minutes) for converting estimates to days of slack.
pub const DEFAULT_WORKDAY_MINUTES: f64 = 480.0;
/// Default horizon in days within which a task's effective slack makes it Urgent in the Eisenhower Matrix.
pub const DEFAULT_URGENT_HORIZON_DAYS: f64 = 2.0;
/// Default minimum `importance` (`1..=5`) for a task to be classified as Important in the Eisenhower Matrix.
pub const DEFAULT_IMPORTANCE_THRESHOLD: u8 = 3;
/// Default top-K limit for the Focus View.
pub const DEFAULT_FOCUS_LIMIT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EisenhowerQuadrant {
    /// Q1: Urgent & Important ("Do First")
    Q1DoFirst,
    /// Q2: Not Urgent & Important ("Schedule")
    Q2Schedule,
    /// Q3: Urgent & Not Important ("Delegate / Quick")
    Q3DelegateQuick,
    /// Q4: Not Urgent & Not Important ("Later")
    Q4Later,
}

impl EisenhowerQuadrant {
    pub const fn from_axes(is_important: bool, is_urgent: bool) -> Self {
        match (is_important, is_urgent) {
            (true, true) => Self::Q1DoFirst,
            (true, false) => Self::Q2Schedule,
            (false, true) => Self::Q3DelegateQuick,
            (false, false) => Self::Q4Later,
        }
    }

    pub const fn number(self) -> u8 {
        match self {
            Self::Q1DoFirst => 1,
            Self::Q2Schedule => 2,
            Self::Q3DelegateQuick => 3,
            Self::Q4Later => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EisenhowerClassification {
    pub task: Task,
    pub effective_deadline: Option<NaiveDate>,
    pub slack_days: Option<f64>,
    pub is_important: bool,
    pub is_urgent: bool,
    pub quadrant: EisenhowerQuadrant,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoringConfig {
    pub workday_minutes: f64,
    pub urgent_horizon_days: f64,
    pub importance_threshold: u8,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self {
            workday_minutes: DEFAULT_WORKDAY_MINUTES,
            urgent_horizon_days: DEFAULT_URGENT_HORIZON_DAYS,
            importance_threshold: DEFAULT_IMPORTANCE_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FocusWeights {
    pub urgency: f64,
    pub importance: f64,
    pub unblocks: f64,
    pub effort: f64,
    pub manual_priority: f64,
    pub parallel_boost: f64,
    pub in_progress_boost: f64,
}

impl FocusWeights {
    pub const fn for_preset(preset: FocusPreset) -> Self {
        match preset {
            FocusPreset::Balanced => Self {
                urgency: 35.0,
                importance: 25.0,
                unblocks: 18.0,
                effort: 10.0,
                manual_priority: 8.0,
                parallel_boost: 6.0,
                in_progress_boost: 8.0,
            },
            FocusPreset::QuickWins => Self {
                urgency: 25.0,
                importance: 15.0,
                unblocks: 15.0,
                effort: 40.0,
                manual_priority: 8.0,
                parallel_boost: 6.0,
                in_progress_boost: 8.0,
            },
            FocusPreset::EatTheFrog => Self {
                urgency: 25.0,
                importance: 30.0,
                unblocks: 18.0,
                effort: -35.0,
                manual_priority: 8.0,
                parallel_boost: 6.0,
                in_progress_boost: 8.0,
            },
            FocusPreset::UnblockOthers => Self {
                urgency: 25.0,
                importance: 20.0,
                unblocks: 45.0,
                effort: 10.0,
                manual_priority: 8.0,
                parallel_boost: 6.0,
                in_progress_boost: 8.0,
            },
        }
    }
}

impl Default for FocusWeights {
    fn default() -> Self {
        Self::for_preset(FocusPreset::Balanced)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreBreakdown {
    pub total_score: f64,
    pub urgency_component: f64,
    pub importance_component: f64,
    pub unblocks_component: f64,
    pub effort_component: f64,
    pub manual_priority_component: f64,
    pub parallel_component: f64,
    pub in_progress_component: f64,
    pub effective_deadline: Option<NaiveDate>,
    pub slack_days: Option<f64>,
    pub unblocks_count: usize,
    pub has_in_progress_parallel_peer: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoredTask {
    pub task: Task,
    pub quadrant: EisenhowerQuadrant,
    pub breakdown: ScoreBreakdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerInfo {
    pub blocker_id: String,
    pub title: String,
    pub origin_kind: OriginKind,
    pub assignee: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedTaskEntry {
    pub task_id: String,
    pub title: String,
    pub status: TaskStatus,
    pub tags: Vec<String>,
    pub blocked_by: Vec<BlockerInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedSummaryGroup {
    pub project_or_tag: String,
    pub blocker_kind: OriginKind,
    pub waiting_on_assignee: String,
    pub count: usize,
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FocusQueueResponse {
    pub user: String,
    pub preset: FocusPreset,
    pub actionable_tasks: Vec<ScoredTask>,
    pub blocked_summary: Vec<BlockedSummaryGroup>,
}

/// Resolves a task's effective deadline as `coalesce(task.sgdb_date, task.due_date, ancestor_effective_deadline)`.
pub fn resolve_effective_deadline(
    task_id: &str,
    tasks_by_id: &HashMap<String, Task>,
    parents_by_child: &HashMap<String, Vec<String>>,
) -> Option<NaiveDate> {
    let task = tasks_by_id.get(task_id)?;
    if let Some(d) = task.sgdb_naive_date().or_else(|| task.due_naive_date()) {
        return Some(d);
    }

    // Walk up ancestor parent_of edges looking for the earliest inherited deadline.
    let mut stack = parents_by_child.get(task_id).cloned().unwrap_or_default();
    let mut visited = std::collections::HashSet::new();
    let mut earliest: Option<NaiveDate> = None;

    while let Some(parent_id) = stack.pop() {
        if !visited.insert(parent_id.clone()) {
            continue;
        }
        if let Some(parent) = tasks_by_id.get(&parent_id) {
            if let Some(d) = parent.sgdb_naive_date().or_else(|| parent.due_naive_date()) {
                earliest = Some(match earliest {
                    Some(prev) => prev.min(d),
                    None => d,
                });
            }
            if let Some(grandparents) = parents_by_child.get(&parent_id) {
                stack.extend(grandparents.iter().cloned());
            }
        }
    }

    earliest
}

/// Computes effective slack in days:
/// `D_slack = (EffectiveDeadline - now) - (time_estimate_minutes / workday_minutes)`
pub fn compute_effective_slack_days(
    effective_deadline: Option<NaiveDate>,
    today: NaiveDate,
    time_estimate_minutes: Option<i64>,
    workday_minutes: f64,
) -> Option<f64> {
    let deadline = effective_deadline?;
    let calendar_days = (deadline - today).num_days() as f64;
    let estimate_days = time_estimate_minutes.unwrap_or(0).max(0) as f64 / workday_minutes.max(1.0);
    Some(calendar_days - estimate_days)
}

/// Classifies a single task into its Eisenhower Matrix quadrant.
pub fn classify_task_eisenhower(
    task: &Task,
    effective_deadline: Option<NaiveDate>,
    today: NaiveDate,
    config: ScoringConfig,
) -> EisenhowerClassification {
    let slack_days = compute_effective_slack_days(effective_deadline, today, task.time_estimate_minutes, config.workday_minutes);
    let is_important = task.importance >= config.importance_threshold;
    let is_urgent = task.urgent_override || slack_days.is_some_and(|slack| slack <= config.urgent_horizon_days);
    let quadrant = EisenhowerQuadrant::from_axes(is_important, is_urgent);

    EisenhowerClassification { task: task.clone(), effective_deadline, slack_days, is_important, is_urgent, quadrant }
}

/// Classifies all incomplete (or optionally all) tasks in the database for the Eisenhower Matrix View.
pub fn list_eisenhower_matrix(
    db: &Database,
    user_filter: Option<&str>,
    today: NaiveDate,
    config: ScoringConfig,
) -> Result<Vec<EisenhowerClassification>> {
    let all_tasks = list_tasks(db)?;
    let tasks_by_id: HashMap<String, Task> = all_tasks
        .iter()
        .cloned()
        .map(|t| (t.id.clone(), t))
        .collect();
    let graph = RelationGraph::load(&db.conn)?;

    let normalized_user = match user_filter {
        Some(u) if !u.trim().is_empty() => Some(normalize_user(u)?),
        _ => None,
    };

    let mut items = Vec::new();
    for task in all_tasks {
        if task.status.is_completed() {
            continue;
        }
        if let Some(ref u) = normalized_user
            && &task.assignee != u
        {
            continue;
        }
        let eff_deadline = resolve_effective_deadline(&task.id, &tasks_by_id, &graph.parents_by_child);
        items.push(classify_task_eisenhower(&task, eff_deadline, today, config));
    }

    Ok(items)
}

/// Continuous urgency curve $U(D_{\text{slack}})$:
/// - Returns `0.0` when `slack_days` is `None` and `urgent_override == false`.
/// - Smoothly rises as `slack_days` approaches `0` (`1 / (1 + slack / 3)` for `slack >= 0`).
/// - Escalates linearly when overdue (`slack_days < 0`: `1.0 + (-slack_days) * 0.35`).
/// - Applies a minimum floor of `1.15` when `urgent_override == true`.
pub fn continuous_urgency_score(slack_days: Option<f64>, urgent_override: bool) -> f64 {
    let base = match slack_days {
        Some(slack) if slack < 0.0 => 1.0 + (-slack) * 0.35,
        Some(slack) => 1.0 / (1.0 + slack / 3.0),
        None => 0.0,
    };

    if urgent_override { base.max(1.15) } else { base }
}

/// Computes the ranked Focus Queue and Blocked Summary for `user`.
pub fn compute_focus_queue(
    db: &Database,
    user: &str,
    preset: FocusPreset,
    custom_weights: Option<FocusWeights>,
    limit: Option<usize>,
    today: NaiveDate,
    config: ScoringConfig,
) -> Result<FocusQueueResponse> {
    let norm_user = normalize_user(user)?;
    let weights = custom_weights.unwrap_or_else(|| FocusWeights::for_preset(preset));
    let top_k = limit.unwrap_or(DEFAULT_FOCUS_LIMIT);

    let all_tasks = list_tasks(db)?;
    let tasks_by_id: HashMap<String, Task> = all_tasks
        .iter()
        .cloned()
        .map(|t| (t.id.clone(), t))
        .collect();
    let graph = RelationGraph::load(&db.conn)?;

    let mut scored = Vec::new();
    for task in &all_tasks {
        if task.assignee != norm_user {
            continue;
        }
        if !matches!(task.status, TaskStatus::Todo | TaskStatus::InProgress) {
            continue;
        }

        // Exclude tasks blocked by any incomplete `blocks` predecessor
        if graph.has_incomplete_blocker(&task.id, &tasks_by_id) {
            continue;
        }

        // Exclude container parent tasks that have incomplete `sub_task_of` (`parent_of`) children
        if graph.has_incomplete_subtask(&task.id, &tasks_by_id) {
            continue;
        }

        let effective_deadline = resolve_effective_deadline(&task.id, &tasks_by_id, &graph.parents_by_child);
        let classification = classify_task_eisenhower(task, effective_deadline, today, config);

        let urgency_norm = continuous_urgency_score(classification.slack_days, task.urgent_override);
        let importance_norm = f64::from(task.importance) / 5.0;

        let unblocks_count = graph.count_transitive_incomplete_unblocks(&task.id, &tasks_by_id);
        let unblocks_norm = (1.0 + unblocks_count as f64).ln();

        // Effort score in [0.0, 1.0]: higher for quick/easy tasks, lower for long/hard tasks.
        // Positive `weights.effort` rewards quick wins; negative `weights.effort` (EatTheFrog) rewards hard/long tasks.
        let difficulty_ease = (6.0 - f64::from(task.difficulty)) / 5.0;
        let duration_ease = match task.time_estimate_minutes {
            Some(mins) if mins > 0 => 1.0 / (1.0 + (mins as f64) / 60.0),
            _ => 0.5,
        };
        let effort_norm = if weights.effort >= 0.0 {
            0.5 * difficulty_ease + 0.5 * duration_ease
        } else {
            // Invert so harder/longer tasks have a larger positive multiplier when multiplied by |weights.effort|
            1.0 - (0.5 * difficulty_ease + 0.5 * duration_ease)
        };

        let manual_norm = (f64::from(task.manual_priority) - 3.0) / 2.0;
        let has_in_progress_parallel_peer = graph.has_in_progress_parallel_peer(&task.id, &tasks_by_id);

        let urgency_component = weights.urgency * urgency_norm;
        let importance_component = weights.importance * importance_norm;
        let unblocks_component = weights.unblocks * unblocks_norm;
        let effort_component = weights.effort.abs() * effort_norm;
        let manual_priority_component = weights.manual_priority * manual_norm;
        let parallel_component = if has_in_progress_parallel_peer { weights.parallel_boost } else { 0.0 };
        let in_progress_component = if task.status == TaskStatus::InProgress { weights.in_progress_boost } else { 0.0 };

        let total_score = urgency_component
            + importance_component
            + unblocks_component
            + effort_component
            + manual_priority_component
            + parallel_component
            + in_progress_component;

        scored.push(ScoredTask {
            task: task.clone(),
            quadrant: classification.quadrant,
            breakdown: ScoreBreakdown {
                total_score,
                urgency_component,
                importance_component,
                unblocks_component,
                effort_component,
                manual_priority_component,
                parallel_component,
                in_progress_component,
                effective_deadline,
                slack_days: classification.slack_days,
                unblocks_count,
                has_in_progress_parallel_peer,
            },
        });
    }

    scored.sort_by(|a, b| {
        b.breakdown
            .total_score
            .total_cmp(&a.breakdown.total_score)
            .then_with(|| b.task.importance.cmp(&a.task.importance))
            .then_with(|| a.task.id.cmp(&b.task.id))
    });
    scored.truncate(top_k);

    let blocked_summary = summarize_blocked_tasks(db, &norm_user)?;

    Ok(FocusQueueResponse { user: norm_user, preset, actionable_tasks: scored, blocked_summary })
}

/// Returns all incomplete tasks assigned to `user` that are currently excluded from their
/// actionable Focus View because of active blockers, incomplete subtasks, or `in_review` status.
pub fn get_blocked_tasks(db: &Database, user: &str, tag_filter: Option<&str>) -> Result<Vec<BlockedTaskEntry>> {
    let norm_user = normalize_user(user)?;
    let norm_tag = match tag_filter {
        Some(t) if !t.trim().is_empty() => Some(normalize_tag(t)?),
        _ => None,
    };

    let all_tasks = list_tasks(db)?;
    let tasks_by_id: HashMap<String, Task> = all_tasks
        .iter()
        .cloned()
        .map(|t| (t.id.clone(), t))
        .collect();
    let graph = RelationGraph::load(&db.conn)?;

    let mut entries = Vec::new();
    for task in &all_tasks {
        if task.assignee != norm_user || task.status.is_completed() {
            continue;
        }
        if let Some(ref tag) = norm_tag
            && !task.tags.iter().any(|t| t == tag)
        {
            continue;
        }

        let active_blockers = graph.collect_active_blockers_for_task(&task.id, &tasks_by_id);
        if active_blockers.is_empty() && task.status != TaskStatus::InReview && task.status != TaskStatus::Stuck {
            continue;
        }

        entries.push(BlockedTaskEntry {
            task_id: task.id.clone(),
            title: task.title.clone(),
            status: task.status,
            tags: task.tags.clone(),
            blocked_by: active_blockers,
        });
    }

    Ok(entries)
}

/// Groups blocked tasks for `user` by tag/project (or `"untagged"`), blocker `origin_kind`,
/// and blocking `assignee` so AI agents can immediately summarize who/what they are waiting on.
pub fn summarize_blocked_tasks(db: &Database, user: &str) -> Result<Vec<BlockedSummaryGroup>> {
    let blocked_entries = get_blocked_tasks(db, user, None)?;
    let mut grouped: BTreeMap<(String, String, String), (OriginKind, Vec<String>)> = BTreeMap::new();

    for entry in blocked_entries {
        let primary_tag = entry
            .tags
            .first()
            .cloned()
            .unwrap_or_else(|| "untagged".to_string());

        if entry.blocked_by.is_empty() {
            let key = (primary_tag, OriginKind::Manual.as_str().to_string(), entry.status.as_str().to_string());
            let bucket = grouped
                .entry(key)
                .or_insert((OriginKind::Manual, Vec::new()));
            if !bucket.1.contains(&entry.task_id) {
                bucket.1.push(entry.task_id.clone());
            }
            continue;
        }

        for blocker in entry.blocked_by {
            let key = (primary_tag.clone(), blocker.origin_kind.as_str().to_string(), blocker.assignee.clone());
            let bucket = grouped
                .entry(key)
                .or_insert((blocker.origin_kind, Vec::new()));
            if !bucket.1.contains(&entry.task_id) {
                bucket.1.push(entry.task_id.clone());
            }
        }
    }

    let summary = grouped
        .into_iter()
        .map(|((project_or_tag, _, waiting_on_assignee), (blocker_kind, task_ids))| BlockedSummaryGroup {
            project_or_tag,
            blocker_kind,
            waiting_on_assignee,
            count: task_ids.len(),
            task_ids,
        })
        .collect();

    Ok(summary)
}

struct RelationGraph {
    blockers_by_target: HashMap<String, Vec<String>>,
    blocked_by_source: HashMap<String, Vec<String>>,
    parents_by_child: HashMap<String, Vec<String>>,
    children_by_parent: HashMap<String, Vec<String>>,
    parallel_peers: HashMap<String, Vec<String>>,
}

impl RelationGraph {
    fn load(conn: &Connection) -> Result<Self> {
        let mut stmt = conn.prepare("SELECT source_id, target_id, relation_type FROM task_relations")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;

        let mut blockers_by_target: HashMap<String, Vec<String>> = HashMap::new();
        let mut blocked_by_source: HashMap<String, Vec<String>> = HashMap::new();
        let mut parents_by_child: HashMap<String, Vec<String>> = HashMap::new();
        let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
        let mut parallel_peers: HashMap<String, Vec<String>> = HashMap::new();

        for row in rows {
            let (src, dst, rel_str) = row?;
            match rel_str.as_str() {
                r if r == RelationType::Blocks.as_str() => {
                    blockers_by_target
                        .entry(dst.clone())
                        .or_default()
                        .push(src.clone());
                    blocked_by_source.entry(src).or_default().push(dst);
                }
                r if r == RelationType::ParentOf.as_str() => {
                    parents_by_child
                        .entry(dst.clone())
                        .or_default()
                        .push(src.clone());
                    children_by_parent.entry(src).or_default().push(dst);
                }
                r if r == RelationType::Parallel.as_str() => {
                    parallel_peers
                        .entry(src.clone())
                        .or_default()
                        .push(dst.clone());
                    parallel_peers.entry(dst).or_default().push(src);
                }
                _ => {}
            }
        }

        Ok(Self { blockers_by_target, blocked_by_source, parents_by_child, children_by_parent, parallel_peers })
    }

    fn has_incomplete_blocker(&self, task_id: &str, tasks_by_id: &HashMap<String, Task>) -> bool {
        self.blockers_by_target
            .get(task_id)
            .is_some_and(|blockers| {
                blockers.iter().any(|b_id| {
                    tasks_by_id
                        .get(b_id)
                        .is_some_and(|t| !t.status.is_completed())
                })
            })
    }

    fn has_incomplete_subtask(&self, task_id: &str, tasks_by_id: &HashMap<String, Task>) -> bool {
        self.children_by_parent
            .get(task_id)
            .is_some_and(|children| {
                children.iter().any(|c_id| {
                    tasks_by_id
                        .get(c_id)
                        .is_some_and(|t| !t.status.is_completed())
                })
            })
    }

    fn has_in_progress_parallel_peer(&self, task_id: &str, tasks_by_id: &HashMap<String, Task>) -> bool {
        self.parallel_peers.get(task_id).is_some_and(|peers| {
            peers.iter().any(|p_id| {
                tasks_by_id
                    .get(p_id)
                    .is_some_and(|t| t.status == TaskStatus::InProgress)
            })
        })
    }

    /// Counts how many incomplete tasks are transitively unblocked (via `blocks` or `sub_task_of`/`parent_of`) by `task_id`.
    fn count_transitive_incomplete_unblocks(&self, task_id: &str, tasks_by_id: &HashMap<String, Task>) -> usize {
        let mut visited = std::collections::HashSet::new();
        let mut stack = Vec::new();

        if let Some(blocked) = self.blocked_by_source.get(task_id) {
            stack.extend(blocked.iter().cloned());
        }
        if let Some(parents) = self.parents_by_child.get(task_id) {
            stack.extend(parents.iter().cloned());
        }

        let mut count = 0usize;
        while let Some(curr) = stack.pop() {
            if curr == task_id || !visited.insert(curr.clone()) {
                continue;
            }
            if let Some(t) = tasks_by_id.get(&curr)
                && !t.status.is_completed()
            {
                count += 1;
                if let Some(next_blocked) = self.blocked_by_source.get(&curr) {
                    stack.extend(next_blocked.iter().cloned());
                }
                if let Some(next_parents) = self.parents_by_child.get(&curr) {
                    stack.extend(next_parents.iter().cloned());
                }
            }
        }

        count
    }

    fn collect_active_blockers_for_task(&self, task_id: &str, tasks_by_id: &HashMap<String, Task>) -> Vec<BlockerInfo> {
        let mut blocker_ids = Vec::new();
        if let Some(direct) = self.blockers_by_target.get(task_id) {
            blocker_ids.extend(direct.iter().cloned());
        }
        if let Some(children) = self.children_by_parent.get(task_id) {
            for child_id in children {
                if !blocker_ids.contains(child_id) {
                    blocker_ids.push(child_id.clone());
                }
            }
        }

        let mut infos = Vec::new();
        for b_id in blocker_ids {
            if let Some(b_task) = tasks_by_id.get(&b_id)
                && !b_task.status.is_completed()
            {
                infos.push(BlockerInfo {
                    blocker_id: b_task.id.clone(),
                    title: b_task.title.clone(),
                    origin_kind: b_task.origin_kind,
                    assignee: b_task.assignee.clone(),
                    status: b_task.status,
                });
            }
        }
        infos
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::db::open_in_memory_db;
    use crate::models::{CreateTaskInput, RelationInputKind};
    use crate::notes::{add_task_note, transition_task_status};
    use crate::relations::link_tasks;
    use crate::tasks::create_task;

    #[test]
    fn classifies_eisenhower_quadrants_for_undated_sgdb_large_estimates_and_overrides() {
        let mut db = open_in_memory_db().unwrap();
        let today = NaiveDate::from_ymd_opt(2099, 11, 1).unwrap();
        let config = ScoringConfig::default();

        // 1. Undated important task -> Q2 (Not Urgent & Important)
        let t_q2 = create_task(
            &mut db,
            CreateTaskInput { title: "Important undated".to_string(), importance: Some(4), ..Default::default() },
            "alice",
        )
        .unwrap();
        let c_q2 = classify_task_eisenhower(&t_q2, None, today, config);
        assert_eq!(c_q2.quadrant, EisenhowerQuadrant::Q2Schedule);
        assert!(!c_q2.is_urgent);
        assert!(c_q2.is_important);

        // 2. Undated low-importance task -> Q4 (Not Urgent & Not Important)
        let t_q4 = create_task(
            &mut db,
            CreateTaskInput { title: "Low importance undated".to_string(), importance: Some(2), ..Default::default() },
            "alice",
        )
        .unwrap();
        let c_q4 = classify_task_eisenhower(&t_q4, None, today, config);
        assert_eq!(c_q4.quadrant, EisenhowerQuadrant::Q4Later);

        // 3. Task with due_date 5 days away (Nov 6) but large time estimate (4 workdays = 1920m)
        // pulls effective slack to 5 - 4 = 1.0 day <= 2.0 urgent horizon -> Q1 (Urgent & Important)
        let t_slack = create_task(
            &mut db,
            CreateTaskInput {
                title: "Big task due in 5 days".to_string(),
                importance: Some(5),
                due_date: Some("2099-11-06".to_string()),
                time_estimate_minutes: Some(1920),
                ..Default::default()
            },
            "alice",
        )
        .unwrap();
        let c_slack = classify_task_eisenhower(&t_slack, t_slack.due_naive_date(), today, config);
        assert_eq!(c_slack.slack_days, Some(1.0));
        assert_eq!(c_slack.quadrant, EisenhowerQuadrant::Q1DoFirst);

        // 4. Urgent override on low-importance undated task -> Q3 (Urgent & Not Important)
        let t_q3 = create_task(
            &mut db,
            CreateTaskInput { title: "Ping response".to_string(), importance: Some(1), urgent_override: Some(true), ..Default::default() },
            "alice",
        )
        .unwrap();
        let c_q3 = classify_task_eisenhower(&t_q3, None, today, config);
        assert_eq!(c_q3.quadrant, EisenhowerQuadrant::Q3DelegateQuick);
    }

    #[test]
    fn filters_blocked_tasks_ranks_overdue_first_and_reorders_by_preset() {
        let mut db = open_in_memory_db().unwrap();
        let today = NaiveDate::from_ymd_opt(2099, 11, 1).unwrap();
        let config = ScoringConfig::default();

        // Blocked task (should be excluded from actionable Focus queue)
        let blocker = create_task(
            &mut db,
            CreateTaskInput { title: "Blocker task".to_string(), assignee: Some("bob".to_string()), ..Default::default() },
            "alice",
        )
        .unwrap();
        let blocked_target = create_task(
            &mut db,
            CreateTaskInput {
                title: "Blocked target".to_string(),
                importance: Some(5),
                assignee: Some("alice".to_string()),
                ..Default::default()
            },
            "alice",
        )
        .unwrap();
        link_tasks(&mut db, &blocker.id, RelationInputKind::Blocks, &blocked_target.id).unwrap();

        // Overdue task vs due-tomorrow task
        let overdue = create_task(
            &mut db,
            CreateTaskInput {
                title: "Overdue task".to_string(),
                due_date: Some("2099-01-01".to_string()),
                importance: Some(3),
                difficulty: Some(3),
                time_estimate_minutes: Some(60),
                assignee: Some("alice".to_string()),
                ..Default::default()
            },
            "alice",
        )
        .unwrap();

        // Past deadlines are rejected at creation, so backdate the row directly to simulate a task that became overdue.
        db.conn
            .execute("UPDATE tasks SET due_date = '2099-10-30' WHERE id = ?1", [&overdue.id])
            .unwrap();

        let due_tomorrow = create_task(
            &mut db,
            CreateTaskInput {
                title: "Due tomorrow".to_string(),
                due_date: Some("2099-01-02".to_string()),
                importance: Some(3),
                difficulty: Some(3),
                time_estimate_minutes: Some(60),
                assignee: Some("alice".to_string()),
                ..Default::default()
            },
            "alice",
        )
        .unwrap();
        db.conn
            .execute("UPDATE tasks SET due_date = '2099-11-02' WHERE id = ?1", [&due_tomorrow.id])
            .unwrap();

        let balanced = compute_focus_queue(&db, "alice", FocusPreset::Balanced, None, None, today, config).unwrap();
        let ids: Vec<&str> = balanced
            .actionable_tasks
            .iter()
            .map(|s| s.task.id.as_str())
            .collect();
        assert!(!ids.contains(&blocked_target.id.as_str()));
        assert_eq!(ids[0], overdue.id);
        assert_eq!(ids[1], due_tomorrow.id);

        // QuickWins vs EatTheFrog preset reordering
        let mut db2 = open_in_memory_db().unwrap();
        let quick_easy = create_task(
            &mut db2,
            CreateTaskInput {
                title: "10m easy fix".to_string(),
                importance: Some(3),
                difficulty: Some(1),
                time_estimate_minutes: Some(10),
                assignee: Some("alice".to_string()),
                ..Default::default()
            },
            "alice",
        )
        .unwrap();

        let hard_frog = create_task(
            &mut db2,
            CreateTaskInput {
                title: "4h deep architecture refactor".to_string(),
                importance: Some(3),
                difficulty: Some(5),
                time_estimate_minutes: Some(240),
                assignee: Some("alice".to_string()),
                ..Default::default()
            },
            "alice",
        )
        .unwrap();

        let qw = compute_focus_queue(&db2, "alice", FocusPreset::QuickWins, None, None, today, config).unwrap();
        assert_eq!(qw.actionable_tasks[0].task.id, quick_easy.id);

        let frog = compute_focus_queue(&db2, "alice", FocusPreset::EatTheFrog, None, None, today, config).unwrap();
        assert_eq!(frog.actionable_tasks[0].task.id, hard_frog.id);
    }

    #[test]
    fn summarizes_blocked_tasks_across_reviews_mentions_and_dependencies() {
        let mut db = open_in_memory_db().unwrap();
        let today = NaiveDate::from_ymd_opt(2099, 11, 1).unwrap();

        // 1. Task in project-a blocked by a @bob mention subtask
        let t_proj_a = create_task(
            &mut db,
            CreateTaskInput {
                title: "Build API client".to_string(),
                tags: vec!["project-a".to_string()],
                assignee: Some("coder".to_string()),
                evaluator: Some("coder".to_string()),
                ..Default::default()
            },
            "coder",
        )
        .unwrap();
        add_task_note(&mut db, &t_proj_a.id, "coder", "Waiting on @bob for schema details", true).unwrap();

        // 2. Task in project-b submitted for review to @human
        let t_proj_b = create_task(
            &mut db,
            CreateTaskInput {
                title: "Design settings dialog".to_string(),
                tags: vec!["project-b".to_string()],
                assignee: Some("coder".to_string()),
                evaluator: Some("human".to_string()),
                ..Default::default()
            },
            "human",
        )
        .unwrap();
        transition_task_status(&mut db, &t_proj_b.id, TaskStatus::Completed).unwrap();

        let focus = compute_focus_queue(&db, "coder", FocusPreset::Balanced, None, None, today, ScoringConfig::default()).unwrap();
        assert!(focus.actionable_tasks.is_empty());
        assert_eq!(focus.blocked_summary.len(), 2);

        let group_a = focus
            .blocked_summary
            .iter()
            .find(|g| g.project_or_tag == "project-a")
            .unwrap();
        assert_eq!(group_a.blocker_kind, OriginKind::Mention);
        assert_eq!(group_a.waiting_on_assignee, "bob");
        assert_eq!(group_a.count, 1);

        let group_b = focus
            .blocked_summary
            .iter()
            .find(|g| g.project_or_tag == "project-b")
            .unwrap();
        assert_eq!(group_b.blocker_kind, OriginKind::Review);
        assert_eq!(group_b.waiting_on_assignee, "human");
        assert_eq!(group_b.count, 1);
    }
}
