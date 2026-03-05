Refactor Villalobos to always start with a Planner agent

  Currently, the app flow is:
    1. App::run(goal) → spawns Orchestrator
    2. Orchestrator decides: call decompose() (→ Planner → child Orchestrator) OR implement() (→ Implementer)

  Change it to:
    1. App::run(goal) → spawns Planner first to create a plan
    2. Then spawns Orchestrator to execute that plan
    3. Orchestrator can still call implement() for simple tasks or decompose() for complex ones
    4. When decompose() is called, it should follow the same pattern: spawn Planner → get plan → spawn child Orchestrator to execute

  Specific changes needed:

    1. `App::run()` in src/app.rs:
       • Remove direct call to run_orchestrator(goal)
       • Instead: call spawn_planner(goal) → wait_for_plan() → then run_orchestrator() with the plan
       • The orchestrator prompt should receive the plan entries to execute, not the raw goal

    2. `handle_decompose_inner()` in src/app.rs:
       • This already spawns a planner then a child orchestrator, so the pattern is correct
       • Just ensure it's consistent with the new root-level flow

    3. Orchestrator prompt in prompts/orchestrator.txt:
       • Update to expect a list of tasks to execute (from the plan) rather than a raw goal
       • It should iterate through tasks, calling implement() or decompose() for each

    4. Remove the decision logic from root orchestrator:
       • Root orchestrator no longer needs to decide "should I decompose or implement the whole thing"
       • It receives a plan and executes it

  Testing:
    • Run cargo build to verify compilation
    • Run cargo test to verify existing tests pass
    • Test manually with: cargo run "Create a hello world Python program"
    • Verify the planner runs first, creates a plan, then orchestrator executes it

  Do NOT change:
    • The MCP server tools (decompose, implement, complete)
    • The ACP client implementation
    • The planner or implementer prompts (unless necessary for the flow)