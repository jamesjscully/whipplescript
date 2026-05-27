# Armature Grammar

Status: v0 implementation sketch

The v0 parser follows the newer BAML language tooling pattern:

```text
source text
  -> logos token stream with spans and trivia
  -> rowan lossless syntax tree
  -> typed AST wrappers / lowering
  -> WorkflowIR
```

The parser must preserve comments, whitespace, malformed tokens, and source
spans in the rowan syntax tree even when lowering fails. The runtime never
executes the syntax tree directly; it executes validated WorkflowIR.

## Lexical Rules

```ebnf
Ident        = (ALPHA | "_") (ALNUM | "_" | "-")*
String       = '"' string_char* '"'
BlockString  = '"""' block_string_char* '"""'
Int          = DIGIT+
Float        = DIGIT+ "." DIGIT+
Duration     = DIGIT+ ("ms" | "s" | "m" | "h" | "d")
LineComment  = "//" (!NEWLINE ANY)*
Trivia       = WHITESPACE | NEWLINE | LineComment
```

Keywords:

```text
machine initial data event agent capability adapter enum class coerce model
prompt state on as guard entry always final invariant let assign start send
askHuman raise stay goto case in nil true false profile
```

## Source File

```ebnf
File =
  MachineDecl
  InitialDecl
  TopLevelDecl*

TopLevelDecl =
    DataBlock
  | EventDecl
  | AgentDecl
  | CapabilityDecl
  | EnumDecl
  | ClassDecl
  | CoerceDecl
  | StateDecl
  | InvariantDecl
```

A file defines exactly one machine. `machine` and top-level `initial` are
required.

## Declarations

```ebnf
MachineDecl = "machine" Ident
InitialDecl = "initial" StateRef

DataBlock = "data" Block<DataField>
DataField = Ident TypeExpr ("=" Expr)?

EventDecl = "event" Ident Block<Field>

AgentDecl =
  "agent" Ident "=" AgentCtor AgentOptions?

AgentCtor =
    "thread" "(" String ")"
  | "codingAgent" ("(" ")")?
  | "adapter" "(" String ")"

AgentOptions =
  Block<AgentOption>

AgentOption =
  "maxActive" Int
  | "profile" String

`maxActive` must be greater than zero. If a workflow starts an agent with a
declared `maxActive` limit, v0 requires a declared `finished` event with a
required `name string` field and at least one `finished` handler. The runtime
uses processed `finished.name` values with agent-name prefixes such as
`worker-01` to retire active invocations.
`profile` names the requested harness authority profile for native agent
execution, such as `"research"` or `"repo-writer"`. Profile names are semantic
intent labels, not provider names; concrete commands, filesystem posture,
network posture, timeout, and enforcement mode are resolved through harness
profile policy.
Thread agents are message targets and cannot be used with `start`. Native
started work must target `codingAgent`. The `codingAgent()` spelling is a
compatibility alias, but canonical source should omit parentheses because this
is a role declaration, not a concrete provider constructor. Explicitly external starts may target
an adapter-backed agent when the adapter contract is loaded and policy permits
it.

CapabilityDecl =
  "capability" Ident "=" "adapter" "(" String ")"

EnumDecl =
  "enum" Ident Block<EnumValue>

EnumValue = Ident

ClassDecl =
  "class" Ident Block<Field>

Field = Ident TypeExpr

CoerceDecl =
  "coerce" Ident "(" ParamList? ")" "->" TypeExpr Block<CoerceMember>

ParamList = Param ("," Param)*
Param = Ident TypeExpr

CoerceMember =
    "model" String
  | "prompt" BlockString

InvariantDecl =
    "invariant" Ident
  | "invariant" Ident "{" "assert" Expr "}"
```

## States

```ebnf
StateDecl =
  "state" Ident "{" StateMember* "}"

StateMember =
    InitialDecl
  | EntryBlock
  | AlwaysBlock
  | OnBlock
  | StateDecl
  | "final"

EntryBlock = "entry" ActionBlock
AlwaysBlock = "always" GuardList? ActionBlock

OnBlock =
  "on" Ident ("as" Ident)? GuardList? ActionBlock

GuardList = Guard+
Guard = "guard" Expr
```

Handler lookup starts at the active leaf state and walks outward through parent
states. Ambiguous unguarded handlers for the same event at the same state level
are validation errors.

## Actions

```ebnf
ActionBlock = "{" Statement* Outcome? "}"

Statement =
    LetStmt
  | AssignStmt
  | StartStmt
  | SendStmt
  | AskHumanStmt
  | RaiseStmt
  | CapabilityCallStmt
  | CaseStmt

LetStmt = "let" Ident "=" Expr
AssignStmt = "assign" Path "=" Expr

StartStmt =
  "start" Ident ObjectBlock?

SendStmt =
  "send" Ident Expr

AskHumanStmt =
  "askHuman" "(" Expr ")"

RaiseStmt =
  "raise" Ident ObjectBlock?

CapabilityCallStmt =
  Ident "." Ident "(" ArgList? ")"

Outcome =
    "stay"
  | "goto" StateRef
```

`stay` is a no-op outcome. `goto` changes state. In v0, a handler may omit an
explicit outcome, which is equivalent to staying in the current state after its
effects complete. `finish`, `fail`, `exit`, `after`, and `parallel` are reserved
for future grammar revisions and are not part of the implemented v0 surface.
If `stay` or `goto` is present, it must be the last statement in that action
block. Repeated explicit outcomes are parse errors.
`StateRef` is currently a single state identifier, resolved against the machine's
declared state names. Dotted or relative state paths are not implemented in v0.

## Case

```ebnf
CaseStmt =
  "case" Expr "{" CaseArm+ "}"

CaseArm =
    Pattern "->" ActionBlock

Pattern =
    Ident
  | Literal
  | "matches" String
  | "_"
```

## Expressions

```ebnf
Expr        = Or
Or          = And ("||" And)*
And         = Equality ("&&" Equality)*
Equality    = Compare (("==" | "!=") Compare)?
Compare     = Membership (("<" | "<=" | ">" | ">=") Membership)?
Membership  = Unary ("in" Unary)?
Unary       = "!" Unary | Primary
Primary     =
    Path
  | Literal
  | Call
  | Object
  | List
  | "(" Expr ")"

Call        = Path "(" ArgList? ")"
ArgList     = Expr ("," Expr)*
Path        = Ident ("." Ident)*
Object      = "{" ObjectField* "}"
ObjectField = Ident Expr
List        = "[" (Expr ("," Expr)*)? "]"
Literal     = String | BlockString | Int | Float | Duration | "true" | "false" | "nil"
```

`!in` may be added as sugar later. The v0 canonical spelling is:

```armature
guard !(run.id in data.seenRuns)
```

## Type Expressions

```ebnf
TypeExpr =
  UnionType

UnionType =
  OptionalType ("|" OptionalType)*

OptionalType =
  PostfixType "?"
  | PostfixType

PostfixType =
  PrimaryType "[]"*

PrimaryType =
    Ident
  | Literal
  | "map" "<" TypeExpr "," TypeExpr ">"
  | "(" TypeExpr ")"
```

Map key type expressions are syntactically general, but v0 validation requires
them to be string-compatible because runtime values are JSON objects. Valid key
schemas are `string`, enums, string literals, and unions/refs composed from
those.

BAML-compatible primitive type names are:

```text
string int float bool null
```

Armature-native type names include:

```text
time duration agent json
```

Native types are valid for workflow data, events, and capability schemas, but
they are not valid coerce input/output boundary types unless a compiler rule or
adapter maps them to BAML-compatible types.

## Implemented V0 Surface

The current parser implementation lowers the grammar above for:

```text
machine
initial
data with typed fields and optional initial values
events with typed fields
agent declarations with maxActive
agent adapter targets
capability declarations
enum and class declarations
coerce declarations
state
entry
always
on ... as ... with guards
let
assign
start
send
askHuman
raise
capability calls
case
stay
goto
final
invariant builtin names and one-assert expression blocks
paths, calls, objects, lists, strings, numbers, bools, nil, durations
optional, list, union, map, literal, native, and reference type expressions
```

Data initial values are parsed as expressions, but v0 validation accepts only
static literal/list/object initializers that match the declared field schema.

The lexer and rowan syntax tree still preserve trivia and malformed tokens so
the grammar can grow without changing the frontend architecture.
