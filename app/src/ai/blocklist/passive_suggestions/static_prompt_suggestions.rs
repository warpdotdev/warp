use std::sync::LazyLock;

use regex::Regex;
use uuid::Uuid;

use crate::terminal::view::PromptSuggestion;

pub struct StaticPromptSuggestion {
    pub name: &'static str,
    pub pattern: &'static str,
    pub label_template: Option<&'static str>,
    pub query_template: &'static str,
}

/// Attempts to match a terminal command against predefined static prompt suggestions.
///
/// If the command matches a static rule, this returns a [`SuggestedQuery`] with details from the
/// command substituted into the rule's query template.
pub fn static_suggested_query(command: &str) -> Option<PromptSuggestion> {
    // Try each rule in turn and apply the first match.
    for pattern in &*RULE_PATTERNS {
        if let Some(captures) = pattern.regex.captures(command) {
            // If there's a match, apply placeholders to the query.
            let label = pattern
                .rule
                .label_template
                .map(|template| apply_captures(template, &captures));
            let query = apply_captures(pattern.rule.query_template, &captures);

            return Some(PromptSuggestion {
                id: Uuid::new_v4().to_string(),
                label,
                prompt: query,
                coding_query_context: None,
                static_prompt_suggestion_name: Some(pattern.rule.name.to_string()),
                should_start_new_conversation: false,
            });
        }
    }

    None
}

/// A static prompt suggestion with its pattern precompiled to a [`Regex`].
struct StaticPromptRule {
    rule: &'static StaticPromptSuggestion,
    regex: Regex,
}

static RULE_PATTERNS: LazyLock<Vec<StaticPromptRule>> = LazyLock::new(|| {
    STATIC_RULES
        .iter()
        .map(|rule| match Regex::new(rule.pattern) {
            Ok(regex) => StaticPromptRule { rule, regex },
            Err(e) => {
                panic!(
                    "Invalid pattern for static prompt rule `{}`: {}",
                    rule.name, e
                );
            }
        })
        .collect()
});

static STATIC_RULES: &[StaticPromptSuggestion] = &[
    // git checkout -b <branch>: Checks out a new branch named <branch>.
    StaticPromptSuggestion {
        name: "GIT_CHECKOUT_NEW_BRANCH",
        pattern: r"^git\s+checkout\s+-b\s+(\S+)\s*$",
        label_template: Some("Code a feature or fix a bug in {1}"),
        query_template: "Implement a feature or fix a bug in {1}. Ask me for all the details you need.",
    },
    // git clone <repo>: Clones a repository named <repo>.
    StaticPromptSuggestion {
        name: "GIT_CLONE",
        pattern: r"^git\s+clone\s+(\S+)\s*$",
        label_template: Some("Help me code a feature or fix a bug in {1}"),
        query_template: "Implement a feature or fix a bug in {1}. Ask me for all the details you need.",
    },
    // git switch -c <branch>: Creates and switches to a new branch named <branch>.
    StaticPromptSuggestion {
        name: "GIT_SWITCH_NEW_BRANCH",
        pattern: r"^git\s+switch\s+-c\s+(\S+)\s*$",
        label_template: Some("Code a feature or fix a bug in {1}"),
        query_template: "Implement a feature or fix a bug in {1}. Ask me for all the details you need.",
    },
    // git push: Pushes changes to a remote repository.
    StaticPromptSuggestion {
        name: "GIT_PUSH",
        pattern: r"^git\s+push\s*$",
        label_template: None,
        query_template: "Help me create a pull request.",
    },
    // git init: Initializes a new, empty Git repository.
    StaticPromptSuggestion {
        name: "GIT_INIT",
        pattern: r"^git\s+init\s*$",
        label_template: Some("Help me start a new project"),
        query_template: "Help me start a new project. Ask me for all the details you need.",
    },
    // npm init / yarn init / pnpm init: Initializes a Node.js project.
    StaticPromptSuggestion {
        name: "NODE_PACKAGE_INIT",
        pattern: r"^(npm|yarn|pnpm)\s+init\s*$",
        label_template: Some("Help me start a Node.js project"),
        query_template: "Help me start a Node.js project. Ask me for all the details you need.",
    },
    // npx create-react-app <project>: Creates a new React app called <project>.
    StaticPromptSuggestion {
        name: "NPX_CREATE_REACT_APP",
        pattern: r"^npx\s+create-react-app\s+(\S+)\s*$",
        label_template: Some("Help me create a new React app"),
        query_template: "Help me create a new React app called {1}. Ask me for all the details you need.",
    },
    // npx create-next-app <project>: Creates a new Next.js app called <project>.
    StaticPromptSuggestion {
        name: "NPX_CREATE_NEXT_APP",
        pattern: r"^npx\s+create-next-app\s+(\S+)\s*$",
        label_template: Some("Help me create a new Next.js app"),
        query_template: "Help me create a new Next.js app called {1}. Ask me for all the details you need.",
    },
    // cargo new <project>: Creates a new Rust package named <project>.
    StaticPromptSuggestion {
        name: "CARGO_NEW_PROJECT",
        pattern: r"^cargo\s+new\s+(\S+)\s*$",
        label_template: Some("Help me start a Rust project for {1}"),
        query_template: "Help me start a Rust project for {1}. Ask me for all the details you need.",
    },
    // poetry new <project>: Creates a new Poetry-based Python project named <project>.
    StaticPromptSuggestion {
        name: "POETRY_NEW_PROJECT",
        pattern: r"^poetry\s+new\s+(\S+)\s*$",
        label_template: Some("Help me start a Poetry project for {1}"),
        query_template: "Help me start a Poetry project for {1}. Ask me for all the details you need.",
    },
    // django-admin startproject <project>: Creates a new Django project named <project>.
    StaticPromptSuggestion {
        name: "DJANGO_START_PROJECT",
        pattern: r"^django-admin\s+startproject\s+(\S+)\s*$",
        label_template: Some("Help me start a Django project for {1}"),
        query_template: "Help me start a Django project for {1}. Ask me for all the details you need.",
    },
    // rails new <app>: Creates a new Rails app named <app>.
    StaticPromptSuggestion {
        name: "RAILS_NEW_APP",
        pattern: r"^rails\s+new\s+(\S+)\s*$",
        label_template: Some("Help me start a Rails app for {1}"),
        query_template: "Help me start a Rails app for {1}. Ask me for all the details you need.",
    },
    // gradle init / mvn archetype:generate: Initializes a Gradle or Maven project.
    StaticPromptSuggestion {
        name: "JAVA_PROJECT_INIT",
        pattern: r"^(gradle\s+init|mvn\s+archetype:generate)\s*$",
        label_template: Some("Help me start a Gradle/Maven project"),
        query_template: "Help me start a Gradle/Maven project. Ask me for all the details you need.",
    },
    // go mod init <module>: Initializes a new Go module named <module>.
    StaticPromptSuggestion {
        name: "GO_MOD_INIT",
        pattern: r"^go\s+mod\s+init\s+(\S+)\s*$",
        label_template: Some("Help me start a Go project for {1}"),
        query_template: "Help me start a Go project for {1}. Ask me for all the details you need.",
    },
    // swift package init: Initializes a new Swift package.
    StaticPromptSuggestion {
        name: "SWIFT_PACKAGE_INIT",
        pattern: r"^swift\s+package\s+init\s*$",
        label_template: Some("Help me start a Swift project"),
        query_template: "Help me start a Swift project. Ask me for all the details you need.",
    },
    // terraform init: Initializes Terraform in the current directory.
    StaticPromptSuggestion {
        name: "TERRAFORM_INIT",
        pattern: r"^terraform\s+init\s*$",
        label_template: Some("Help me start a Terraform configuration"),
        query_template: "Help me start a Terraform configuration. Ask me for all the details you need.",
    },
    // prisma init: Initializes Prisma in the current project.
    StaticPromptSuggestion {
        name: "PRISMA_INIT",
        pattern: r"^prisma\s+init\s*$",
        label_template: Some("Help me set up Prisma in this project"),
        query_template: "Help me set up Prisma in this project.",
    },
    // python -m venv <env_name>: Creates a new Python virtual environment named <env_name>.
    StaticPromptSuggestion {
        name: "PYTHON_CREATE_VENV",
        pattern: r"^python\s+-m\s+venv\s+(\S+)\s*$",
        label_template: None,
        query_template: "Help me install dependencies for {1}.",
    },
    // bundle init: Creates a new Gemfile (Ruby Bundler).
    StaticPromptSuggestion {
        name: "BUNDLE_INIT",
        pattern: r"^bundle\s+init\s*$",
        label_template: Some("Help me set up a new Ruby project"),
        query_template: "Help me set up a new Ruby project. Ask me for all the details you need.",
    },
    // ollama pull <model>: Pulls an Ollama model named <model>.
    StaticPromptSuggestion {
        name: "OLLAMA_PULL_MODEL",
        pattern: r"^ollama\s+pull\s+(\S+)\s*$",
        label_template: None,
        query_template: "Help me set up a Modelfile for {1}.",
    },
    // kubectl top nodes: Shows node resource usage in Kubernetes.
    StaticPromptSuggestion {
        name: "KUBECTL_TOP_NODES",
        pattern: r"^kubectl\s+top\s+(nodes|node|no)\s*$",
        label_template: None,
        query_template: "Help me understand resource utilization in my cluster.",
    },
    // kubectl top pods: Shows pod resource usage in Kubernetes.
    StaticPromptSuggestion {
        name: "KUBECTL_TOP_PODS",
        pattern: r"^kubectl\s+top\s+(pods|po|pod)\s*$",
        label_template: None,
        query_template: "Help me understand resource utilization in my cluster.",
    },
    // kubectl get...: Gets Kubernetes resources (any).
    StaticPromptSuggestion {
        name: "KUBECTL_GET_RESOURCES",
        pattern: r"^kubectl\s+get.*$",
        label_template: None,
        query_template: "Help me inspect Kubernetes resources.",
    },
    // docker ps: Lists Docker containers.
    StaticPromptSuggestion {
        name: "DOCKER_LIST_CONTAINERS",
        pattern: r"^docker\s+ps\s*$",
        label_template: None,
        query_template: "Help me manage running containers.",
    },
    // docker image ls: Lists Docker images.
    StaticPromptSuggestion {
        name: "DOCKER_LIST_IMAGES",
        pattern: r"^docker\s+image\s+ls\s*$",
        label_template: None,
        query_template: "Help me manage Docker images.",
    },
    // docker-compose up -d <service>: Spins up a service <service> in Docker Compose.
    StaticPromptSuggestion {
        name: "DOCKER_COMPOSE_UP_SERVICE",
        pattern: r"^docker-compose\s+up\s+-d\s+(\S+)\s*$",
        label_template: Some("Help me manage or troubleshoot {1} with Docker Compose"),
        query_template: "Help me manage or troubleshoot {1} with Docker Compose.",
    },
    // docker network create <network>: Creates a Docker network named <network>.
    StaticPromptSuggestion {
        name: "DOCKER_NETWORK_CREATE",
        pattern: r"^docker\s+network\s+create\s+(\S+)\s*$",
        label_template: None,
        query_template: "Help me configure containers to use {1}.",
    },
    // vagrant init <box>: Initializes a Vagrant box named <box>.
    StaticPromptSuggestion {
        name: "VAGRANT_INIT_BOX",
        pattern: r"^vagrant\s+init\s+(\S+)\s*$",
        label_template: None,
        query_template: "Help me set up or customize a Vagrant box {1}.",
    },
    // vagrant up: Brings up a Vagrant environment.
    StaticPromptSuggestion {
        name: "VAGRANT_UP",
        pattern: r"^vagrant\s+up\s*$",
        label_template: None,
        query_template: "Help me provision my environment or troubleshoot Vagrant startup.",
    },
    // grep -r <pattern>: Searches recursively for <pattern> in files.
    StaticPromptSuggestion {
        // Capture everything after `grep -r ` into capture group 1.
        name: "GREP_RECURSIVE_SEARCH",
        pattern: r"^grep\s+-r\s+(.*)$",
        label_template: None,
        query_template: "Help me search code across files for {1}.",
    },
    // find <args>: Searches for files/directories using `find`.
    StaticPromptSuggestion {
        // Capture everything after `find ` into capture group 1.
        // E.g. `find . -name "*.rs"`.
        name: "FIND_FILES",
        pattern: r"^find\s+(.*)$",
        label_template: None,
        query_template: "Help me search code across files with {1}.",
    },
    // ssh-keygen (no args): Generates an SSH key with default options.
    StaticPromptSuggestion {
        // This pattern matches "ssh-keygen" by itself or anything after it (e.g. "-t rsa -b 4096").
        name: "SSH_KEYGEN",
        pattern: r"^ssh-keygen(?:\s+(.*))?$",
        // We’ll keep the label/query generic so it applies whether or not the user passed extra flags.
        // Not using the capture group here, but it's there if we need it for the future.
        label_template: None,
        query_template: "Walk me through generating an SSH key.",
    },
];

pub fn apply_captures(template: &str, captures: &regex::Captures) -> String {
    // We'll look for placeholders of the form `{1}`, `{2}`, etc. and replace them with the
    // corresponding capture group.
    let mut result = String::from(template);

    for i in 1..captures.len() {
        let placeholder = format!("{{{i}}}");
        if let Some(m) = captures.get(i) {
            result = result.replace(&placeholder, m.as_str());
        }
    }
    result
}
