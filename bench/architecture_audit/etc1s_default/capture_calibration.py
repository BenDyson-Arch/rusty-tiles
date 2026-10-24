"""Diagnostic-only instrumentation. Apply before measurement, remove before shipping.
No gate or codec behavior changes; writes actual first-attempt channel distributions.
Usage: python3 capture_calibration.py [--remove]
"""
from pathlib import Path
import sys
ROOT = Path(__file__).resolve().parents[3]
BEGIN = "// BEGIN ETC1S CALIBRATION\n"
END = "// END ETC1S CALIBRATION\n"
chunks = {
 "src/stage_timing.rs": ("", r'''pub(crate) fn calibration_record(value: serde_json::Value) {
    use std::io::Write;
    if let Some(path) = std::env::var_os("RT_CODEC_CALIBRATION") {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap();
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(file, "{value}").unwrap();
    }
}
'''),
 "src/mesh_large/leaves/delivery.rs": ("        tamper(step, index, &mut decoded);\n", r'''        if std::env::var_os("RT_CODEC_CALIBRATION").is_some() {
            for channel in 0..4 {
                let stats = channel_stats(&prepared.expected[index], &decoded, &level.support, level.width as usize, channel);
                timing::calibration_record(json!({"kind":"level","slot":format!("{slot:?}"),"profile":format!("{profile:?}"),"step":step.name(),"mip":index,"width":level.width,"channel":channel,"psnr":stats.psnr,"p999":stats.p999,"max":stats.max,"blockRmse":stats.block_rmse,"texels":stats.texels}));
            }
        }
'''),
 "src/mesh_large/leaves/source_sampler_mips.rs": ("            if psnr < floor || maximum[a] > cap {\n", r'''            if std::env::var_os("RT_CODEC_CALIBRATION").is_some() {
                crate::stage_timing::calibration_record(serde_json::json!({"kind":"sampler","slot":format!("{slot:?}"),"profile":format!("{profile:?}"),"lod":lod,"channel":a,"psnr":psnr,"max":maximum[a]}));
            }
'''),
}
for name, (anchor, code) in chunks.items():
    p = ROOT / name
    text = p.read_text()
    if BEGIN in text:
        start = text.index(BEGIN)
        end = text.index(END, start) + len(END)
        text = text[:start] + text[end:]
    if "--remove" not in sys.argv:
        block = BEGIN + code + END
        if not anchor:
            text += "\n" + block
        elif "source_sampler" in name:
            text = text.replace(anchor, block + anchor, 1)
        else:
            text = text.replace(anchor, anchor + block, 1)
    p.write_text(text)
