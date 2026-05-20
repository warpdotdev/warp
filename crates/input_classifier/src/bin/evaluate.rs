use std::{fs, process};

use clap::{Parser, Subcommand};
use futures::executor::block_on;
use input_classifier::{
    ClassificationResult, Context, HeuristicClassifier, InputClassifier, InputType,
    test_utils::CompletionContext,
};

/// Convert HSL to RGB values (0-255 range)
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r_prime, g_prime, b_prime) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    let r = ((r_prime + m) * 255.0) as u8;
    let g = ((g_prime + m) * 255.0) as u8;
    let b = ((b_prime + m) * 255.0) as u8;

    (r, g, b)
}

/// Generate ANSI color code for smooth mode using HSL saturation scaling
fn get_smooth_confidence_color(is_correct: bool, confidence: f32) -> String {
    // Map confidence (0.5 to 1.0) to saturation (0.0 to 1.0)
    // Confidence below 0.5 gets 0 saturation (gray), above 0.5 scales linearly
    let saturation = if confidence <= 0.5 {
        0.0
    } else {
        (confidence - 0.5) * 2.0
    };

    // Use different hues for correct vs incorrect
    let hue = if is_correct { 120.0 } else { 0.0 }; // Green for correct, Red for incorrect
    let lightness = 0.5; // Medium lightness

    let (r, g, b) = hsl_to_rgb(hue, saturation, lightness);
    format!("\x1b[38;2;{r};{g};{b}m")
}

/// Generate ANSI color code for binary mode using simple green/red with dim for low confidence
fn get_binary_confidence_color(is_correct: bool, is_low_confidence: bool) -> String {
    if is_correct {
        if is_low_confidence {
            "\x1b[32m\x1b[2m".to_string() // Green + dim for correct but low confidence
        } else {
            "\x1b[32m".to_string() // Green for correct
        }
    } else if is_low_confidence {
        "\x1b[31m\x1b[2m".to_string() // Red + dim for incorrect but low confidence
    } else {
        "\x1b[31m".to_string() // Red for incorrect
    }
}
use warp_completer::{ParsedTokensSnapshot, util::parse_current_commands_and_tokens};

#[cfg(feature = "onnx")]
use input_classifier::{OnnxClassifier, OnnxModel};
#[cfg(feature = "onnx")]
fn default_onnx_model() -> Option<OnnxModel> {
    cfg_if::cfg_if! {
        if #[cfg(feature = "nld_classifier_v1")] {
            Some(OnnxModel::BertTinyV1)
        } else if #[cfg(feature = "nld_classifier_v2")] {
            Some(OnnxModel::BertTinyV2)
        } else {
            None
        }
    }
}

#[derive(Parser)]
struct InputSource {
    /// Input string to classify (use --file to read from file instead)
    #[arg(group = "input_source")]
    input: Option<String>,
    /// Read input from file instead of command line argument
    #[arg(long, group = "input_source")]
    file: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ConfidenceMode {
    Binary(f32), // Binary mode with confidence threshold
    Smooth,      // Smooth saturation scaling
}

#[derive(Parser)]
#[command(name = "evaluate")]
#[command(about = "Test input classifier implementations")]
struct Args {
    /// Use heuristic classifier
    #[arg(long)]
    heuristic: bool,

    /// Use ONNX classifier
    #[cfg(feature = "onnx")]
    #[arg(long)]
    onnx: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Classify a single input string
    Classify {
        #[command(flatten)]
        input_source: InputSource,
    },
    /// Verify classification by testing all prefixes of input string
    Verify {
        expected: String,
        #[command(flatten)]
        input_source: InputSource,
        /// Confidence threshold for binary mode visualization. If specified, uses binary coloring with the given threshold (e.g., 0.9). If not specified, uses smooth saturation scaling.
        #[arg(long)]
        confident: Option<f32>,
    },
}

/// Create classifiers based on CLI flags
fn create_classifiers(args: &Args) -> Vec<(&'static str, Box<dyn InputClassifier>)> {
    let mut classifiers: Vec<(&'static str, Box<dyn InputClassifier>)> = Vec::new();

    // Default to all available classifiers if none specified
    let onnx_specified = {
        #[cfg(feature = "onnx")]
        {
            args.onnx
        }
        #[cfg(not(feature = "onnx"))]
        {
            false
        }
    };

    let use_all = !args.heuristic && !onnx_specified;

    if args.heuristic || use_all {
        classifiers.push(("heuristic", Box::new(HeuristicClassifier)));
    }

    #[cfg(feature = "onnx")]
    if args.onnx || use_all {
        let Some(model) = default_onnx_model() else {
            eprintln!("Warning: No ONNX model feature is enabled for the ONNX classifier");
            return classifiers;
        };
        match OnnxClassifier::new(model) {
            Ok(classifier) => {
                classifiers.push(("onnx", Box::new(classifier)));
            }
            Err(e) => {
                eprintln!("Warning: Failed to initialize ONNX classifier: {e}");
            }
        }
    }

    classifiers
}

/// Resolve input from either direct string or file
fn resolve_input_source(input: Option<String>, file: Option<String>) -> anyhow::Result<String> {
    match (input, file) {
        (Some(input_str), None) => Ok(input_str),
        (None, Some(file_path)) => {
            let content = fs::read_to_string(file_path.clone())
                .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", file_path, e))?;
            // Trim trailing newline if present
            Ok(content.trim_end().to_string())
        }
        (Some(_), Some(_)) => Err(anyhow::anyhow!("Cannot specify both input string and file")),
        (None, None) => Err(anyhow::anyhow!(
            "Must specify either input string or --file"
        )),
    }
}

/// Parse input string into ParsedTokensSnapshot
async fn parse_input(input: &str) -> anyhow::Result<ParsedTokensSnapshot> {
    let completion_context = CompletionContext::new();
    let snapshot = parse_current_commands_and_tokens(input.to_string(), &completion_context).await;
    Ok(snapshot)
}

/// Handle classify command
async fn handle_classify(
    input: &str,
    classifiers: &[(&str, Box<dyn InputClassifier>)],
) -> anyhow::Result<()> {
    let parsed_input = parse_input(input).await?;
    let context = Context {
        current_input_type: InputType::Shell,
        is_agent_follow_up: false,
    };

    println!("Input: \"{input}\"");
    println!("Classifications:");

    for (name, classifier) in classifiers {
        match classifier
            .classify_input(parsed_input.clone(), &context)
            .await
        {
            Ok(result) => {
                let predicted_type = result.to_input_type();
                println!(
                    "  {}: {} (p_shell: {:.3}, p_ai: {:.3}, confidence: {:.3}, opacity: {:.3})",
                    name,
                    predicted_type,
                    result.p_shell(),
                    result.p_ai(),
                    result.confidence(),
                    opacity(&result)
                );
            }
            Err(_) => {
                // Fallback to detect_input_type if classify_input fails
                let result = classifier
                    .detect_input_type(parsed_input.clone(), &context)
                    .await;
                println!("  {name}: {result} (probabilities unavailable)");
            }
        }
    }

    Ok(())
}

/// Handle verify command (tests all prefixes)
async fn handle_verify(
    input: &str,
    expected: InputType,
    classifiers: &[(&str, Box<dyn InputClassifier>)],
    confidence_mode: ConfidenceMode,
) -> anyhow::Result<()> {
    println!("Input: \"{input}\"");
    println!("Expected: {expected}");
    println!("Verification Results:");

    // Generate all prefixes (1 character to full string)
    let prefixes: Vec<String> = (1..=input.len()).map(|i| input[..i].to_string()).collect();

    println!("Testing {} prefixes...", prefixes.len());

    for (name, classifier) in classifiers {
        let mut correct_count = 0;
        let total_count = prefixes.len();
        let mut classification_results: Vec<(bool, Option<ClassificationResult>)> = Vec::new();

        for prefix in &prefixes {
            let parsed_input = parse_input(prefix).await?;
            let context = Context {
                current_input_type: InputType::Shell,
                is_agent_follow_up: false,
            };

            // Use classify_input to get probabilities
            let classification_result = classifier
                .classify_input(parsed_input.clone(), &context)
                .await;

            let (is_correct, result_opt) = match classification_result {
                Ok(result) => {
                    let predicted_type = result.to_input_type();
                    let is_correct = predicted_type == expected;
                    if is_correct {
                        correct_count += 1;
                    }
                    (is_correct, Some(result))
                }
                Err(_) => {
                    // Fallback to detect_input_type if classify_input fails
                    let result = classifier
                        .detect_input_type(parsed_input.clone(), &context)
                        .await;
                    let is_correct = result == expected;
                    if is_correct {
                        correct_count += 1;
                    }
                    (is_correct, None)
                }
            };

            classification_results.push((is_correct, result_opt));
        }

        let percentage = (correct_count as f64 / total_count as f64) * 100.0;
        println!("  {name}: {correct_count}/{total_count} correct ({percentage:.1}%)");

        print!("    Visual: ");
        for (i, ch) in input.chars().enumerate() {
            let (is_correct, classification_result) = &classification_results[i];

            // Determine styling based on correctness and confidence
            if let Some(result) = classification_result {
                let color_code = match confidence_mode {
                    ConfidenceMode::Smooth => {
                        let confidence = result.confidence();
                        get_smooth_confidence_color(*is_correct, confidence)
                    }
                    ConfidenceMode::Binary(threshold) => {
                        let confidence = result.confidence();
                        let is_low_confidence = confidence < threshold;
                        get_binary_confidence_color(*is_correct, is_low_confidence)
                    }
                };
                print!("{color_code}{ch}\x1b[0m");
            } else {
                // Fallback to basic colors when no probability data is available
                if *is_correct {
                    print!("\x1b[32m{ch}\x1b[0m"); // Green for correct
                } else {
                    print!("\x1b[31m{ch}\x1b[0m"); // Red for incorrect
                }
            }
        }
        println!(); // New line after the colored string
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let classifiers = create_classifiers(&args);
    if classifiers.is_empty() {
        eprintln!("Error: No classifiers available or selected");
        process::exit(1);
    }

    let result: anyhow::Result<()> = block_on(async {
        match &args.command {
            Command::Classify { input_source } => {
                let input_str =
                    resolve_input_source(input_source.input.clone(), input_source.file.clone())?;
                handle_classify(&input_str, &classifiers).await
            }
            Command::Verify {
                expected,
                input_source,
                confident,
            } => {
                let input_str =
                    resolve_input_source(input_source.input.clone(), input_source.file.clone())?;
                let expected_type = expected
                    .parse::<InputType>()
                    .map_err(|e| anyhow::anyhow!("{}", e))?;
                let confidence_mode = match confident {
                    Some(threshold) => {
                        if *threshold < 0.0 || *threshold > 1.0 {
                            return Err(anyhow::anyhow!(
                                "Confidence threshold must be between 0.0 and 1.0, got: {}",
                                threshold
                            ));
                        }
                        ConfidenceMode::Binary(*threshold)
                    }
                    None => ConfidenceMode::Smooth,
                };
                handle_verify(&input_str, expected_type, &classifiers, confidence_mode).await
            }
        }
    });

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }

    Ok(())
}

/// Returns opacity value (0.0 to 1.0) scaled from confidence, where 0.5 confidence = 0.0 opacity and 1.0 confidence = 1.0 opacity
fn opacity(result: &ClassificationResult) -> f32 {
    let conf = result.confidence();
    if conf <= 0.5 { 0.0 } else { (conf - 0.5) * 2.0 }
}
