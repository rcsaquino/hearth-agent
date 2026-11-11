//! Provider presets and onboarding copy.

pub struct ProviderPreset {
    pub label: &'static str,
    pub base_url: Option<&'static str>,
}

pub const PROVIDERS: [ProviderPreset; 5] = [
    ProviderPreset {
        label: "DeepSeek",
        base_url: Some("https://api.deepseek.com/v1"),
    },
    ProviderPreset {
        label: "OpenCode",
        base_url: Some("https://opencode.ai/inference/openai/v1"),
    },
    ProviderPreset {
        label: "OpenCode Go",
        base_url: Some("https://opencode.ai/inference/go/openai/v1"),
    },
    ProviderPreset {
        label: "Command Code",
        base_url: Some("https://api.commandcode.ai/provider/v1"),
    },
    ProviderPreset {
        label: "Other OpenAI-compatible",
        base_url: None,
    },
];

pub const THINKING: [(&str, &str); 7] = [
    ("default", "Provider default"),
    ("off", "Off"),
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra high"),
    ("max", "Maximum"),
];

pub fn key_hint(base_url: &str) -> &'static str {
    if base_url.contains("deepseek.com") {
        "Create a DeepSeek key at https://platform.deepseek.com/api_keys"
    } else if base_url.contains("opencode.ai/inference/go") {
        "OpenCode Go uses your OpenCode key from https://opencode.ai/auth"
    } else if base_url.contains("opencode") {
        "Create an OpenCode key at https://opencode.ai/auth"
    } else if base_url.contains("commandcode.ai") {
        "Create a Command Code key at https://commandcode.ai/settings/keys"
    } else {
        "Use the API key for your provider."
    }
}
