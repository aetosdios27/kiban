QUERY InitAgent(agent: U32) =>
    a <- AddN<Agent>({agent_num: agent})
    RETURN a

// W1. A single committed memory per request; no batching.
QUERY Remember(memory_id: String, agent: U32, created_at: U64, revision: U64, text: String) =>
    memory <- AddN<Memory>({memory_id: memory_id, agent: agent, created_at: created_at, revision: revision, text: text})
    a <- N<Agent>({agent_num: agent})
    edge <- AddE<Remembers>::From(a)::To(memory)
    RETURN "ok"

// W2, write read-back verification, and the read side of W5.
QUERY Recall(memory_id: String) =>
    memory <- N<Memory>({memory_id: memory_id})
    RETURN memory

// W3. Indexed agent lookup, adjacency traversal, server-side ordering and limit.
QUERY Recent(agent: U32, k: U32) =>
    memories <- N<Agent>({agent_num: agent})::Out<Remembers>::ORDER<Desc>(_::{created_at})::RANGE(0, k)
    RETURN memories

// W4 and the write side of W5. Identity and creation time stay unchanged.
QUERY Update(memory_id: String, revision: U64, text: String) =>
    memory <- N<Memory>({memory_id: memory_id})::UPDATE({revision: revision, text: text})
    RETURN "ok"

// Same HTTP server, router and read transaction; no graph/storage access.
QUERY Noop() =>
    RETURN "agentbench-noop"
