"""Diagnostic-only authored sweep input capture; remove hooks before shipping.
Usage: python3 capture_pages.py [--remove]. RT_SWEEP_DUMP selects the output.
All gates/encoded pixels remain unchanged. Dumps exact caller mips, expected
RGBA and u32 support, including proxy mips that cannot be rebuilt from a base.
"""
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[3]
path = root / 'src/mesh_large/leaves/delivery.rs'
begin = '// BEGIN ETC1S PAGE CAPTURE\n'
end = '// END ETC1S PAGE CAPTURE\n'
text = path.read_text()
if begin in text:
    start = text.index(begin)
    stop = text.index(end, start) + len(end)
    text = text[:start] + text[stop:]
if '--remove' not in sys.argv:
    anchor = '    let (mode, params) = step_settings(slot, profile, step)?;\n'
    assert text.count(anchor) == 1
    code = r'''    if step == Step::Profile && class(slot) == Class::Colour {
        if let Some(root) = std::env::var_os("RT_SWEEP_DUMP") {
            fn dump(path: std::path::PathBuf, bytes: &[u8]) {
                use std::io::Write;
                use std::os::fd::AsRawFd;
                let mut file = std::fs::File::create(path).unwrap();
                file.write_all(bytes).unwrap();
                file.sync_data().unwrap();
                // Diagnostic output must not consume the tiler's resident
                // memory lease as accumulated dirty/clean filesystem cache.
                assert_eq!(unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) }, 0);
            }
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::path::PathBuf::from(root).join(format!("{id:04}"));
            std::fs::create_dir_all(&dir).unwrap();
            let mut metadata = Vec::new();
            for (index, level) in chain.levels.iter().enumerate() {
                dump(dir.join(format!("{index}.rgba")), &prepared.delivered[index]);
                dump(dir.join(format!("{index}.expected")), &prepared.expected[index]);
                let support: Vec<u8> = level.support.iter().flat_map(|v| v.to_le_bytes()).collect();
                dump(dir.join(format!("{index}.support")), &support);
                metadata.push(json!({"width":level.width,"height":level.height}));
            }
            std::fs::write(dir.join("page.json"), serde_json::to_vec(&json!({"profile":format!("{profile:?}"),"slot":format!("{slot:?}"),"levels":metadata})).unwrap()).unwrap();
        }
    }
'''
    text = text.replace(anchor, begin + code + end + anchor)
path.write_text(text)
