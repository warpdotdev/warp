use super::*;

lazy_static! {
    pub static ref TEST_LAYER: Layer = Layer {
        name: "TestLayer",
        hasher_seeds: (1, 2),
        traffic_allocations: HashMap::from([
            (TestExperiment::Control.get_group_id(), 10.0,),
            (TestExperiment::Experiment.get_group_id(), 10.0,),
            (FooExperiment::Control.get_group_id(), 5.0,),
            (FooExperiment::Experiment.get_group_id(), 5.0,),
        ]),
        bucket_ranges: vec![
            BucketRange::new(TestExperiment::Control, 0..100),
            BucketRange::new(TestExperiment::Experiment, 300..400),
            BucketRange::new(FooExperiment::Control, 500..550),
            BucketRange::new(FooExperiment::Experiment, 550..600),
        ],
    };
}

const TEST_EXPERIMENT_CONTROL: &str = "Control";
const TEST_EXPERIMENT_EXPERIMENT: &str = "Experiment";

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum TestExperiment {
    Control,
    Experiment,
}

impl Experiment<TestExperiment> for TestExperiment {
    fn name() -> &'static str {
        "TestExperiment"
    }

    fn variant(&self) -> &'static str {
        match self {
            Self::Control => TEST_EXPERIMENT_CONTROL,
            Self::Experiment => TEST_EXPERIMENT_EXPERIMENT,
        }
    }

    fn allow_user_overrides_in_stable() -> bool {
        false
    }
}

impl FromStr for TestExperiment {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            TEST_EXPERIMENT_CONTROL => Ok(Self::Control),
            TEST_EXPERIMENT_EXPERIMENT => Ok(Self::Experiment),
            _ => Err(anyhow::anyhow!(
                "Variant {} is not a valid group in TestExperiment",
                s
            )),
        }
    }
}

const FOO_EXPERIMENT_CONTROL: &str = "Control";
const FOO_EXPERIMENT_EXPERIMENT: &str = "Experiment";

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum FooExperiment {
    Control,
    Experiment,
}

impl Experiment<FooExperiment> for FooExperiment {
    fn name() -> &'static str {
        "FooExperiment"
    }

    fn layer() -> &'static Layer {
        &TEST_LAYER
    }

    fn variant(&self) -> &'static str {
        match self {
            Self::Control => FOO_EXPERIMENT_CONTROL,
            Self::Experiment => FOO_EXPERIMENT_EXPERIMENT,
        }
    }

    fn allow_user_overrides_in_stable() -> bool {
        true
    }
}

impl FromStr for FooExperiment {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            FOO_EXPERIMENT_CONTROL => Ok(Self::Control),
            FOO_EXPERIMENT_EXPERIMENT => Ok(Self::Experiment),
            _ => Err(anyhow::anyhow!(
                "Variant {} is not a valid group in FooExperiment",
                s
            )),
        }
    }
}

#[test]
fn test_get_group_for_bucket_finds_correct_range() {
    let bucket_range = TEST_LAYER.bucket_ranges.get(1).unwrap();
    let expected = Some(bucket_range.group);
    let lower_bound = bucket_range.range.start;
    let upper_bound_inclusive = bucket_range.range.end - 1;

    let res = TEST_LAYER.get_group_for_bucket(lower_bound);
    assert_eq!(res, expected);
    let res = TEST_LAYER.get_group_for_bucket((lower_bound + upper_bound_inclusive) / 2);
    assert_eq!(res, expected);
    let res = TEST_LAYER.get_group_for_bucket(upper_bound_inclusive);
    assert_eq!(res, expected);
}

#[test]
fn test_get_group_for_bucket_not_in_range() {
    let expected = None;
    let bucket_range = TEST_LAYER.bucket_ranges.get(1).unwrap();
    let lower_bound = bucket_range.range.start;
    let upper_bound = bucket_range.range.end;

    let res = TEST_LAYER.get_group_for_bucket(lower_bound - 1);
    assert_eq!(res, expected);
    let res = TEST_LAYER.get_group_for_bucket(upper_bound);
    assert_eq!(res, expected);
}

#[test]
fn test_from_group_id_errors_if_incorrect_experiment() {
    let group_id = GroupId {
        experiment: FooExperiment::name(),
        variant: FOO_EXPERIMENT_CONTROL,
    };
    let res = TestExperiment::from_group_id(group_id);

    assert!(res.is_err())
}

#[test]
fn test_create_experiment_layer_mappings() {
    let layers = vec![&*TEST_LAYER];
    create_experiment_layer_mappings(&layers);

    assert_eq!(EXPERIMENT_LAYER_MAPPINGS.len(), 2);
    assert_eq!(
        EXPERIMENT_LAYER_MAPPINGS
            .get(TestExperiment::name())
            .expect("Layer mapping should have been created for TestExperiment.")
            .name(),
        TEST_LAYER.name()
    );
    assert_eq!(
        EXPERIMENT_LAYER_MAPPINGS
            .get(FooExperiment::name())
            .expect("Layer mapping should have been created for FooExperiment.")
            .name(),
        TEST_LAYER.name()
    );
}

#[test]
fn test_allow_user_overrides() {
    // Case 1: Experiment and channel don't allow overrides.
    assert!(!TestExperiment::can_use_user_override(Channel::Stable));

    // Case 2: Experiment doesn't allow overrides, channel does.
    assert!(TestExperiment::can_use_user_override(Channel::Dev));

    // Case 3: Experiment and channel allow overrides.
    assert!(FooExperiment::can_use_user_override(Channel::Dev));

    // Case 4: Experiment allows overrides, channel doesn't.
    assert!(FooExperiment::can_use_user_override(Channel::Stable));
}
