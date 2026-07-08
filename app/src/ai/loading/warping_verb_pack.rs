//! Built-in verb packs for the custom "warping" spinner.
//!
//! Each pack is a curated list of short flavor phrases. Users (or the agent,
//! via the bundled `modify-settings` skill) can apply a pack by selecting its
//! identifier in the `agents.warp_agent.spinner_verbs` setting.
//!
//! Packs are stored as static string slices without trailing ellipses so they
//! pair cleanly with the render-time normalization, which appends "..." to any
//! verb that does not already end with punctuation.

/// A preset pack of flavor verbs for the warping spinner.
///
/// Referenced by the Settings UI and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarpingVerbPack {
    Medieval,
    ConspiracyTheorist,
    Cooking,
    Warpy,
}

impl WarpingVerbPack {
    /// Returns every built-in pack, in display order.
    pub const fn all() -> &'static [WarpingVerbPack] {
        &[
            WarpingVerbPack::Medieval,
            WarpingVerbPack::ConspiracyTheorist,
            WarpingVerbPack::Cooking,
            WarpingVerbPack::Warpy,
        ]
    }

    /// Human-readable name suitable for a Settings UI button.
    pub const fn display_name(self) -> &'static str {
        match self {
            WarpingVerbPack::Medieval => "Medieval",
            WarpingVerbPack::ConspiracyTheorist => "Conspiracy",
            WarpingVerbPack::Cooking => "Cooking",
            WarpingVerbPack::Warpy => "Warpy",
        }
    }

    /// The list of verbs in this pack. Verbs are stored without trailing
    /// ellipses; rendering normalization adds them.
    pub const fn verbs(self) -> &'static [&'static str] {
        match self {
            WarpingVerbPack::Medieval => &[
                "At your service, my liege",
                "At once, my lord",
                "The scribes set to work",
                "Seeking wisdom from the realm",
                "Consulting the ancient tomes",
                "Dispatching riders across the kingdom",
                "Draining the flagons",
                "Interrogating the lesser lords",
                "Raising the drawbridge",
                "Rallying the bannermen",
            ],
            WarpingVerbPack::ConspiracyTheorist => &[
                "Questioning science",
                "Conspiring",
                "Speculating",
                "Melting steel beams",
                "Confirmation biasing",
                "Doing my own research",
                "Looking for alternative facts",
                "Waking up the sheep",
                "Internet deep diving",
                "Gathering evidence",
                "Proceeding with skepticism",
            ],
            WarpingVerbPack::Cooking => &[
                "Sautéing",
                "Caramelizing",
                "Slicing and dicing",
                "Bruleeing",
                "Flambéing",
                "Immersion blending",
                "Sous viding",
                "Emulsifying",
                "Fermenting",
                "Braising",
            ],
            WarpingVerbPack::Warpy => &[
                "Warping",
                "Going to infinity",
                "Gaining speed",
                "Morphing",
                "Wormhole-ing",
                "Orbiting",
                "Galaxy braining",
                "Shooting stars",
                "Nebulizing",
                "Constellating",
            ],
        }
    }

    /// Returns the pack's verbs allocated as a `Vec<String>`, convenient for
    /// previews and tests.
    pub fn verbs_as_vec(self) -> Vec<String> {
        self.verbs().iter().map(|v| (*v).to_string()).collect()
    }
}

#[cfg(test)]
#[path = "warping_verb_pack_tests.rs"]
mod tests;
