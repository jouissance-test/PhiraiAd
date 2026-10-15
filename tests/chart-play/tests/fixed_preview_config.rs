use phira_chart_play_tests::{
    config::{Config, FIXED_PREVIEW_TIME_DEFAULT, FIXED_PREVIEW_TIME_MAX, FIXED_PREVIEW_TIME_STEP},
    practice_speed,
};


#[test]
fn fixed_preview_defaults_to_450_ms_with_millisecond_adjustment() {
    assert_eq!(FIXED_PREVIEW_TIME_DEFAULT, 0.45);
    assert_eq!(FIXED_PREVIEW_TIME_STEP, 0.001);
    let config: Config = serde_json::from_str("{}").unwrap();
    assert_eq!(config.fixed_preview_time, 0.45);
}

#[test]
fn fixed_preview_value_keeps_and_displays_millisecond_precision() {
    let mut config: Config = serde_json::from_str(r#"{"fixedPreviewTime":0.451}"#).unwrap();
    config.init();
    assert_eq!(config.fixed_preview_time, 0.451);
    assert_eq!(practice_speed::display_for_step(config.fixed_preview_time, FIXED_PREVIEW_TIME_STEP), "0.451");
    assert_eq!(practice_speed::display_for_step(0.45, FIXED_PREVIEW_TIME_STEP), "0.450");
}

#[test]
fn legacy_long_preview_values_are_capped_when_loading_and_reading() {
    assert_eq!(FIXED_PREVIEW_TIME_MAX, 0.75);
    for value in [0.751, 0.8, 2.4, 5.0] {
        let mut config: Config = serde_json::from_value(serde_json::json!({
            "fixedPreviewTimeEnabled": true,
            "fixedPreviewTime": value,
        })).unwrap();
        assert_eq!(config.fixed_preview_seconds(), 0.75);
        config.init();
        assert!(config.fixed_preview_time_enabled);
        assert_eq!(config.fixed_preview_time, 0.75);
    }
}
