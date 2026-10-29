use losscape_core::ids::RunId;
use losscape_core::proto::v0::{LogBatch, ScalarColumns};
use prost::Message;

#[test]
fn log_batch_round_trip() {
    let run_id = RunId::new();
    let batch = LogBatch {
        run_id: run_id.as_bytes().to_vec(),
        seq: 7,
        scalars: Some(ScalarColumns {
            keys: vec!["train/loss".into(), "lr".into()],
            key_idx: vec![0, 1, 0],
            step: vec![1, 1, 2],
            time_ms: vec![1_000, 1_000, 1_010],
            value: vec![0.9, 1e-3, f64::NAN],
        }),
        ..Default::default()
    };

    let decoded = LogBatch::decode(batch.encode_to_vec().as_slice()).unwrap();
    assert_eq!(RunId::from_bytes(&decoded.run_id), Some(run_id));
    assert_eq!(decoded.seq, 7);
    let scalars = decoded.scalars.unwrap();
    assert_eq!(scalars.key_idx, [0, 1, 0]);
    assert!(scalars.value[2].is_nan(), "NaN must survive the wire");
}
