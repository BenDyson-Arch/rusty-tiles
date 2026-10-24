"""Measurement-only H17 stage admission, always inside the owner's hard cap.

Usage: python3 capture_budget.py [--remove]. Never ship the inserted block.
The automatic half-headroom budget refuses tex3's retained child packets.
Use an explicit 8GiB stage allowance only under the verified10GiB/zero-swap
scope. Ownership charges, codec/gates, outputs and escalation stay intact.
This is not evidence that the shipping automatic budget accepts the bake.
"""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[3]
path = root / 'src/mesh_large/bake/integration.rs'
begin = '// BEGIN ETC1S MEASUREMENT BUDGET\n'
end = '// END ETC1S MEASUREMENT BUDGET\n'
text = path.read_text()
if begin in text:
    start = text.index(begin)
    stop = text.index(end, start) + len(end)
    text = text[:start] + text[stop:]
if '--remove' not in sys.argv:
    anchor = "        // The source sampler's own peak for its largest image: decoded chain at 24 bytes\n"
    assert text.count(anchor) == 1
    code = r'''        let memory_bytes = {
            let membership = std::fs::read_to_string("/proc/self/cgroup").unwrap();
            let relative = membership.lines().find_map(|l| l.strip_prefix("0::")).unwrap();
            let scope = std::path::Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/'));
            assert_eq!(std::fs::read_to_string(scope.join("memory.max")).unwrap().trim(), "10737418240");
            assert_eq!(std::fs::read_to_string(scope.join("memory.swap.max")).unwrap().trim(), "0");
            eprintln!("measurement-only H17 stage budget: automatic={memory_bytes}, explicit=8589934592; hard cap=10737418240");
            8u64 << 30
        };
'''
    text = text.replace(anchor, begin + code + end + anchor)
path.write_text(text)
