// A union type to express common command token delimiters. This is used by various 
// fields throughout the schema.
type Delimiter = ',' | ':' | ';' | '/' | ' ' | '::';

// Add a new wrapper struct around the root Command object, which scales to provide a 
// semantically correct place for whole-command level configuration properties, like
// supported CLI versions, signature author(s), way to resolve conflicts among CLIs with 
// the same name. 
interface CommandSignature {
  command: Command;

// See More configurable options parsing below.
  parseOptions?: {
    optionArgumentDelimiters?: Delimiter[];
    optionsMustPrecedeArguments?: boolean;
    flagsArePosixNoncompliant?: boolean;
  };
}

// This is passed to generateAdditionalSuggestions and custom generator implementations.
// See Support dynamic commands and Custom generators below.
interface CompletionContext {  
  // Tokens in the input buffer.
  tokens: string[];

  // Path to shell executable.
  shell: string;

  // The session's current working directory.
  pwd: string;
  
  // Way to execute arbitrary shell command.
  // Maybe restrict this to running in restricted mode? How do we ensure this is safe?
  executeShellCommand: (command: string) => Promise<{
    exitCode: number;
    success: boolean;
    output: string;
  }>;
}

interface Command {
  name: string;
  alias?: string | string[];

  // See Configure suggestion accept behavior below.
  insertValue?: string;

  description?: string;
  arguments?: Argument | Argument[];
  subcommands?: Command[];
  options?: Option[];
  priority?: number;

  // When true, `subcommands` are treated as a set of repeatable, order-independent keywords
  // (e.g. `iif`, `from`, `table` under `ip rule add iif eth0 from 10.0.0.0/8 table main`)
  // rather than a set of mutually exclusive subcommands. Once one keyword has been used, its
  // still-unused sibling keywords remain eligible for suggestion alongside it. Defaults to
  // false, so ordinary exclusive subcommand trees are unaffected.
  repeatableKeywords?: boolean;

  // See Support dynamic subcommands below.
  runtimeOptionsAndSubcommands?: (ctx: CompletionContext) => {
  	options?: Option[];
    subcommands?: Command[];
  };
}

interface Argument {
  name: string;
  description?: string;
  // This is renamed from ArgumentType to ArgumentValue, because that's semantically what 
  // this array is -- a collection of suggestions for argument values.
  values?: ArgumentValue[];
  optional?: boolean;

  // See Alias support below.
  expandAlias?: (ctx: CompletionContext) => Promise<string[]>;

  // See Multi-suggestion arguments below.
  arity?: {
    limit?: number | undefined,
    delimiter?: Delimiter[],
  }, 
}

declare enum TemplateType {
  Files = "TemplateType.Files",
  Folders = "TemplateType.Folders" ,
  FilesAndFolders = "TemplateType.FilesAndFolders",
}

interface Template {
  typeName: TemplateType;
  filterName?: string;
}

interface ShellCommandGeneratorFn {
  script: string | ((tokens: string[]) => string);

  // If left unspecified, splits the output of script on newlines by default. Splitting 
  // on newlines by default is new behavior (currently the postProcess function is 
  // required).
  postProcess?: (script_output: string) => GeneratorResults;
}

// See Custom generators below.
type CustomGeneratorFn = (ctx: CompletionContext) => Promise<GeneratorResults>;
type GeneratorFn = ShellCommandGeneratorFn | CustomGeneratorFn;

interface SuggestionGenerator {
  generateSuggestionsFn: GeneratorFn,

  options?: {
    customTrigger?: string
  }
}

interface GeneratorResults {
  suggestions: Suggestion[];
  is_ordered?: boolean;
}

declare type RootCommand = {
  is_root_command: true
};

type ArgumentValue = Suggestion
  | Template
  | SuggestionGenerator
  | RootCommand;

interface Suggestion {
  value: string;
  displayValue?: string;
  description?: string;
  priority?: number;
  icon?: IconType;
  isHidden?: boolean;

  insertValue?: string;

  dangerous?: boolean;
  deprecated?: boolean;
}

declare enum IconType {
  File = "IconType.File",
  Folder = "IconType.Folder",
  GitBranch = "IconType.GitBranch",
}

interface Option {
  name: string | string[];

  insertValue?: string;
  description?: string;
  arguments?: Argument | Argument[];
  required?: boolean;
  priority?: number;

  dangerous?: boolean;
  deprecated?: boolean;

  requiredOptions?: string[];
  incompatibleOptions?: string[];
  repeatable?: boolean;
}

