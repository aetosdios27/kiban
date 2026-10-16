N::Agent {
    UNIQUE INDEX agent_num: U32
}
N::Memory {
    UNIQUE INDEX memory_id: String,
    agent: U32,
    created_at: U64,
    revision: U64,
    text: String
}
E::Remembers {
    From: Agent,
    To: Memory,
    Properties: {}
}
