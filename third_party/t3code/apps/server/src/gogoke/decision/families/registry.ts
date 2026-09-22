export type DecisionFamily =
  | "RESOURCE_SELECTION"
  | "SESSION_LIFECYCLE"
  | "CONTEXT_SELECTION"
  | "MEMORY_LIFECYCLE"
  | "RISK_CLASSIFICATION"
  | "ESCALATION"
  | "ATTENTION"
  | "OPTIMIZATION";

export type S1DecisionMode = "fixture_bounded_auto" | "proposal_or_replay";

export interface DecisionScenarioDefinition {
  readonly id: string;
  readonly key: string;
  readonly family: DecisionFamily;
  readonly purpose: string;
  readonly ceiling: string;
  readonly s1Mode: S1DecisionMode;
  readonly liveStatus: "NOT_QUALIFIED";
}

const scenario = (
  id: string,
  key: string,
  family: DecisionFamily,
  purpose: string,
  ceiling: string,
  s1Mode: S1DecisionMode,
): DecisionScenarioDefinition =>
  Object.freeze({ id, key, family, purpose, ceiling, s1Mode, liveStatus: "NOT_QUALIFIED" as const });

export const DECISION_SCENARIOS: ReadonlyArray<DecisionScenarioDefinition> = Object.freeze([
  scenario("DF01","seat_composition","RESOURCE_SELECTION","复用/创建职责席位","existing delegated RoleSpec ceiling","proposal_or_replay"),
  scenario("DF02","execution_recipe","RESOURCE_SELECTION","运行端/实例/账号引用/模型/工具/隔离配方","qualified candidates only","fixture_bounded_auto"),
  scenario("DF03","task_granularity","RESOURCE_SELECTION","合并拆分及串并行建议","DAG/write scopes unchanged","proposal_or_replay"),
  scenario("DF04","priority_capacity","RESOURCE_SELECTION","资源/优先级","numeric constraints in code","proposal_or_replay"),
  scenario("DF05","evidence_value","CONTEXT_SELECTION","下一个取证步骤的信息价值","read-only allowed observation","proposal_or_replay"),
  scenario("DF06","blocker_classification","ESCALATION","故障/阻塞类别","not actual kill/retry command","proposal_or_replay"),
  scenario("DF07","review_allocation","RISK_CLASSIFICATION","安排审查资源","cannot remove required reviews","proposal_or_replay"),
  scenario("DF08","owner_attention","ATTENTION","通知/批准聚合","cannot hide mandatory failures","proposal_or_replay"),
  scenario("DF09","session_lifecycle","SESSION_LIFECYCLE","复用/恢复/分叉/新建/重建/归档","cleanliness/custody hard-filter","fixture_bounded_auto"),
  scenario("DF10","context_relevance","CONTEXT_SELECTION","材料相关性","mandatory constraints retained","fixture_bounded_auto"),
  scenario("DF11","context_freshness","CONTEXT_SELECTION","陈旧/冲突候选","source facts verified in code","proposal_or_replay"),
  scenario("DF12","compression_coverage","CONTEXT_SELECTION","摘要必要信息覆盖","generation independent; source preserved","proposal_or_replay"),
  scenario("DF13","context_affinity","RESOURCE_SELECTION","席位已有上下文适配","privacy before relevance","proposal_or_replay"),
  scenario("DF14","project_memory","MEMORY_LIFECYCLE","项目记忆候选","no source deletion","proposal_or_replay"),
  scenario("DF15","global_memory","MEMORY_LIFECYCLE","跨项目提升/回用","explicit cross-domain grant","proposal_or_replay"),
  scenario("DF16","question_view_optimization","OPTIMIZATION","优化Jev问题/视图","proposal only, sealed holdout","proposal_or_replay"),
  scenario("DF17","invocation_optimization","OPTIMIZATION","是否调用/批次/路由/阈值","budget and policy unchanged","proposal_or_replay"),
  scenario("DF18","downstream_dream_loop","OPTIMIZATION","下游评价/校准/梦境闭环","Jev never sole outcome judge","proposal_or_replay"),
]);

const byId = new Map(DECISION_SCENARIOS.map((entry) => [entry.id, entry] as const));

export function decisionScenario(id: string): DecisionScenarioDefinition | null {
  return byId.get(id) ?? null;
}
