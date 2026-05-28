def descend($segments):
  reduce $segments[] as $segment (
    .;
    if type == "object" and has($segment) then .[$segment] else null end
  );

def unique_preserve_order:
  reduce .[] as $item ([]; if index($item) == null then . + [$item] else . end);

def profile_group_names:
  .groups // [];

def has_profile_features:
  type == "object" and (has("groups") or has("features"));

def has_unsupported_base_group:
  type == "object" and has("base");

def resolve_profile($package; $profile_path):
  ($package.metadata.warp.build_profiles | descend($profile_path | split("."))) as $profile
  | if $profile == null then
      error("Missing Cargo build profile: \($profile_path)")
    elif ($profile | has_unsupported_base_group) then
      error("Cargo build profile uses unsupported base build group; use groups instead: \($profile_path)")
    elif ($profile | has_profile_features | not) then
      error("Cargo build profile path is not a leaf build profile: \($profile_path)")
    else
      ($package.metadata.warp.build_feature_groups // {}) as $feature_groups
      | ($profile | profile_group_names) as $group_names
      | reduce $group_names[] as $group_name (
          { features: [], missing_groups: [] };
          if ($feature_groups | has($group_name)) then
            .features += ($feature_groups[$group_name] // [])
          else
            .missing_groups += [$group_name]
          end
        ) as $group_result
      | if ($group_result.missing_groups | length) > 0 then
          error("Cargo build profile \($profile_path) references undefined build feature groups: \($group_result.missing_groups | join(", "))")
        else
          (($group_result.features + ($profile.features // [])) | unique_preserve_order) as $features
          | ($package.features | keys) as $available_features
          | [
              $features[]
              | select((. as $feature | $available_features | index($feature)) | not)
            ] as $missing_features
          | if ($missing_features | length) > 0 then
              error("Cargo build profile \($profile_path) references undefined Cargo features: \($missing_features | join(", "))")
            else
              $features | join(",")
            end
        end
    end;

.packages[]
| select(.name == "warp")
| resolve_profile(.; $profile_path)
