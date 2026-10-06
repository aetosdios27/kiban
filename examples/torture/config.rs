use std::path::PathBuf;
#[derive(Clone, Debug)]
pub struct Config {
    pub mode: String,
    pub db: PathBuf,
    pub output: PathBuf,
    pub seed: u64,
    pub cycles: u64,
    pub operations: u64,
    pub keys: u64,
    pub sync_every: u64,
    pub fault_percent: u64,
    pub threads: usize,
    pub snapshots: bool,
}
impl Config {
    pub fn parse() -> Result<Self, String> {
        let mut c = Self {
            mode: "simulated".into(),
            db: "torture-db".into(),
            output: "torture-results".into(),
            seed: 42,
            cycles: 1000,
            operations: 200,
            keys: 1000,
            sync_every: 17,
            fault_percent: 70,
            threads: 4,
            snapshots: true,
        };
        let args: Vec<_> = std::env::args().skip(1).collect();
        let mut seen = std::collections::HashSet::new();
        for p in args.chunks(2) {
            if p.len() != 2 || !seen.insert(&p[0]) {
                return Err("options require one value; duplicates forbidden".into());
            }
            let n = || {
                p[1].parse::<u64>()
                    .map_err(|_| format!("invalid value for {}", p[0]))
            };
            match p[0].as_str() {
                "--mode" => c.mode = p[1].clone(),
                "--db" => c.db = (&p[1]).into(),
                "--output" => c.output = (&p[1]).into(),
                "--seed" => c.seed = n()?,
                "--cycles" => c.cycles = n()?,
                "--operations" => c.operations = n()?,
                "--keys" => c.keys = n()?,
                "--sync-every" => c.sync_every = n()?,
                "--fault-percent" => c.fault_percent = n()?,
                "--snapshots" => {
                    c.snapshots = p[1]
                        .parse()
                        .map_err(|_| "--snapshots requires true or false")?
                }
                "--threads" => {
                    c.threads = usize::try_from(n()?).map_err(|_| "threads exceeds usize")?
                }
                _ => return Err(format!("unknown option {}", p[0])),
            }
        }
        if !["simulated", "process", "probes"].contains(&c.mode.as_str())
            || c.cycles == 0
            || c.operations == 0
            || c.keys == 0
            || c.sync_every == 0
            || c.fault_percent > 100
            || c.threads == 0
            || c.threads > 4096
            || c.operations > u64::MAX / c.cycles
        {
            return Err("invalid torture configuration".into());
        }
        Ok(c)
    }
}
pub const HELP: &str = "--mode simulated|process|probes --db DIR --output DIR --seed N --cycles N --operations N --keys N --sync-every N --fault-percent 0..100 --threads N --snapshots true|false
Simulated: existing thread-local device/fault framework, direct Kiban, seeded faults + power loss.
Process: shared-engine concurrent child workload; acknowledged rounds synced before receipt, killed without destructors.
Operations is maximum per cycle; seeded crash point varies. Database must be empty; never deleted.";
