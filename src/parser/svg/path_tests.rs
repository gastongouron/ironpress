use super::path::{read_four, read_number, read_pair, read_six, tokenize_path};
use super::{parse_path_data, parse_points, PathCommand};

#[test]
fn parse_path_data_move_and_line() {
    let commands = parse_path_data("M 0 0 L 10 10");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(commands[1], PathCommand::LineTo(10.0, 10.0));
}

#[test]
fn parse_path_data_cubic() {
    let commands = parse_path_data("M 0 0 C 10 0 10 10 0 10");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(
        commands[1],
        PathCommand::CubicTo(10.0, 0.0, 10.0, 10.0, 0.0, 10.0)
    );
}

#[test]
fn parse_path_data_close() {
    let commands = parse_path_data("M 0 0 L 10 0 L 10 10 Z");
    assert_eq!(commands.len(), 4);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(commands[1], PathCommand::LineTo(10.0, 0.0));
    assert_eq!(commands[2], PathCommand::LineTo(10.0, 10.0));
    assert_eq!(commands[3], PathCommand::ClosePath);
}

#[test]
fn parse_path_data_relative() {
    let commands = parse_path_data("M 0 0 l 10 10");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(commands[1], PathCommand::LineTo(10.0, 10.0));
}

#[test]
fn parse_path_data_horizontal_vertical() {
    let commands = parse_path_data("M 0 0 H 10 V 10");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(commands[1], PathCommand::LineTo(10.0, 0.0));
    assert_eq!(commands[2], PathCommand::LineTo(10.0, 10.0));
}

#[test]
fn parse_points_basic() {
    let points = parse_points("10,20 30,40");
    assert_eq!(points, vec![(10.0, 20.0), (30.0, 40.0)]);
}

#[test]
fn parse_points_space_only() {
    let points = parse_points("10 20 30 40");
    assert_eq!(points, vec![(10.0, 20.0), (30.0, 40.0)]);
}

#[test]
fn parse_points_odd_count() {
    let points = parse_points("10,20,30");
    assert_eq!(points, vec![(10.0, 20.0)]);
}

#[test]
fn parse_points_empty() {
    assert!(parse_points("").is_empty());
}

#[test]
fn parse_points_extra_whitespace() {
    let points = parse_points("  1 , 2  ,  3 , 4  ");
    assert_eq!(points, vec![(1.0, 2.0), (3.0, 4.0)]);
}

#[test]
fn parse_path_relative_move() {
    let commands = parse_path_data("m 5 10 l 3 4");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], PathCommand::MoveTo(5.0, 10.0));
    assert_eq!(commands[1], PathCommand::LineTo(8.0, 14.0));
}

#[test]
fn parse_path_relative_h_v() {
    let commands = parse_path_data("M 10 20 h 5 v 10");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0], PathCommand::MoveTo(10.0, 20.0));
    assert_eq!(commands[1], PathCommand::LineTo(15.0, 20.0));
    assert_eq!(commands[2], PathCommand::LineTo(15.0, 30.0));
}

#[test]
fn parse_path_relative_cubic() {
    let commands = parse_path_data("M 10 10 c 5 0 5 5 0 5");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], PathCommand::MoveTo(10.0, 10.0));
    assert_eq!(
        commands[1],
        PathCommand::CubicTo(15.0, 10.0, 15.0, 15.0, 10.0, 15.0)
    );
}

#[test]
fn parse_path_smooth_cubic_absolute() {
    let commands = parse_path_data("M 0 0 C 10 0 20 10 20 20 S 30 40 20 40");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(
        commands[1],
        PathCommand::CubicTo(10.0, 0.0, 20.0, 10.0, 20.0, 20.0)
    );
    assert_eq!(
        commands[2],
        PathCommand::CubicTo(20.0, 30.0, 30.0, 40.0, 20.0, 40.0)
    );
}

#[test]
fn parse_path_smooth_cubic_relative() {
    let commands = parse_path_data("M 10 10 C 15 10 20 15 20 20 s 5 10 0 10");
    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands[2],
        PathCommand::CubicTo(20.0, 25.0, 25.0, 30.0, 20.0, 30.0)
    );
}

#[test]
fn parse_path_quad_absolute() {
    let commands = parse_path_data("M 0 0 Q 10 20 30 40");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[1], PathCommand::QuadTo(10.0, 20.0, 30.0, 40.0));
}

#[test]
fn parse_path_quad_relative() {
    let commands = parse_path_data("M 10 10 q 5 10 15 20");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[1], PathCommand::QuadTo(15.0, 20.0, 25.0, 30.0));
}

#[test]
fn parse_path_smooth_quad_absolute() {
    let commands = parse_path_data("M 0 0 Q 10 20 20 20 T 40 0");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[2], PathCommand::QuadTo(30.0, 20.0, 40.0, 0.0));
}

#[test]
fn parse_path_smooth_quad_relative() {
    let commands = parse_path_data("M 0 0 Q 5 10 10 10 t 10 0");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[2], PathCommand::QuadTo(15.0, 10.0, 20.0, 10.0));
}

#[test]
fn parse_path_lowercase_z() {
    let commands = parse_path_data("M 0 0 L 10 0 z");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[2], PathCommand::ClosePath);
}

#[test]
fn parse_path_implicit_lineto_after_move() {
    let commands = parse_path_data("M 0 0 10 10 20 20");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(commands[1], PathCommand::LineTo(10.0, 10.0));
    assert_eq!(commands[2], PathCommand::LineTo(20.0, 20.0));
}

#[test]
fn parse_path_implicit_lineto_after_relative_move() {
    let commands = parse_path_data("m 0 0 10 10");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], PathCommand::MoveTo(0.0, 0.0));
    assert_eq!(commands[1], PathCommand::LineTo(10.0, 10.0));
}

#[test]
fn parse_path_negative_numbers() {
    let commands = parse_path_data("M -5 -10 L -20 -30");
    assert_eq!(commands[0], PathCommand::MoveTo(-5.0, -10.0));
    assert_eq!(commands[1], PathCommand::LineTo(-20.0, -30.0));
}

#[test]
fn parse_path_numbers_without_space() {
    let commands = parse_path_data("M10-20L30-40");
    assert_eq!(commands[0], PathCommand::MoveTo(10.0, -20.0));
    assert_eq!(commands[1], PathCommand::LineTo(30.0, -40.0));
}

#[test]
fn parse_path_decimal_without_leading_zero() {
    let commands = parse_path_data("M .5 .5 L 1.5 1.5");
    assert_eq!(commands[0], PathCommand::MoveTo(0.5, 0.5));
    assert_eq!(commands[1], PathCommand::LineTo(1.5, 1.5));
}

#[test]
fn parse_path_consecutive_decimals() {
    let commands = parse_path_data("M 0.5.5 1.5.5");
    assert_eq!(commands[0], PathCommand::MoveTo(0.5, 0.5));
    assert_eq!(commands[1], PathCommand::LineTo(1.5, 0.5));
}

#[test]
fn parse_path_empty() {
    assert!(parse_path_data("").is_empty());
}

#[test]
fn parse_path_unknown_command_skipped() {
    let commands = parse_path_data("M 0 0 A 1 1 0 0 1 10 10 L 20 20");
    assert!(commands.contains(&PathCommand::MoveTo(0.0, 0.0)));
}

#[test]
fn tokenize_path_commas_and_spaces() {
    let tokens = tokenize_path("M10,20 L30,40");
    let expected = ["M", "10", "20", "L", "30", "40"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    assert_eq!(tokens, expected);
}

#[test]
fn tokenize_path_negative_after_number() {
    let tokens = tokenize_path("M10-20");
    let expected = ["M", "10", "-20"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    assert_eq!(tokens, expected);
}

#[test]
fn tokenize_path_double_dot() {
    let tokens = tokenize_path("0.5.5");
    let expected = ["0.5", ".5"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    assert_eq!(tokens, expected);
}

#[test]
fn read_number_past_end() {
    let tokens = Vec::<String>::new();
    let mut index = 0;
    assert!(read_number(&tokens, &mut index).is_none());
}

#[test]
fn read_number_non_numeric() {
    let tokens = vec!["abc".to_string()];
    let mut index = 0;
    assert!(read_number(&tokens, &mut index).is_none());
}

#[test]
fn read_pair_insufficient_tokens() {
    let tokens = vec!["5".to_string()];
    let mut index = 0;
    assert!(read_pair(&tokens, &mut index).is_none());
}

#[test]
fn read_four_insufficient_tokens() {
    let tokens = vec!["1".to_string(), "2".to_string(), "3".to_string()];
    let mut index = 0;
    assert!(read_four(&tokens, &mut index).is_none());
}

#[test]
fn read_six_insufficient_tokens() {
    let tokens = vec![
        "1".to_string(),
        "2".to_string(),
        "3".to_string(),
        "4".to_string(),
        "5".to_string(),
    ];
    let mut index = 0;
    assert!(read_six(&tokens, &mut index).is_none());
}
