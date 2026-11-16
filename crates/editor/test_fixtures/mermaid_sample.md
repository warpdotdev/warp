# Mermaid Render Test
This file is meant to exercise a broad variety of Mermaid diagram types in Warp.
Most of the examples below are adapted from `crates/mermaid_to_svg/samples`.

## Flowchart: shapes
```mermaid
graph TD
    A[Rect] --> B(Rounded)
    B --> C{Diamond}
    C --> D((Circle))
    D --> E([Stadium])
    E --> F[(Cylinder)]
    F --> G[[Subroutine]]
    G --> H{{Hexagon}}
    H --> I>Asymmetric]
```

## Flowchart: styles
```mermaid
graph TD
    A[Start] --> B[Process]
    B --> C{Decision}
    C -->|Yes| D[Success]
    C -->|No| E[Error]
    D --> F[End]
    E --> F
    style A fill:#90EE90
    style B fill:#87CEEB
    style C fill:#FFD700
    style D fill:#98FB98
    style E fill:#FF6B6B
    style F fill:#DDA0DD
```

## Sequence diagram
```mermaid
sequenceDiagram
    participant Alice
    participant Bob

    Note over Alice,Bob: Simple request/response with retry

    Alice->>Bob: Request
    alt success
        Bob-->>Alice: 200 OK
    else transient failure
        Bob-->>Alice: 503 Retry-After
        loop retry up to 3 times
            Alice->>Bob: Request
        end
    end
```

## Class diagram
```mermaid
classDiagram
    direction LR

    class HttpClient

    class ApiClient {
        +request()
    }

    class IService {
        <<interface>>
        +request()
    }

    IService <|.. ApiClient
    ApiClient --> HttpClient : uses
```

## Entity relationship diagram
```mermaid
erDiagram
    CUSTOMER ||--o{ ORDER : places
    ORDER ||--|{ LINE_ITEM : contains

    CUSTOMER {
        string name
        string custNumber
    }

    ORDER {
        int orderNumber
        date orderDate
    }

    LINE_ITEM {
        int quantity
        float price
    }
```

## State diagram
```mermaid
stateDiagram-v2
    [*] --> Validate
    Validate --> IsValid

    state IsValid <<choice>>

    IsValid --> Success : valid
    IsValid --> Failure : invalid

    state Fork <<fork>>
    Success --> Fork
    Fork --> TaskA
    Fork --> TaskB

    state Join <<join>>
    TaskA --> Join
    TaskB --> Join
    Join --> [*]

    Failure --> [*]
```

## Journey diagram
```mermaid
journey
    title Sprint planning

    section Morning
      Stand up: 5: Alice, Bob
      Review backlog: 3: Alice
      Estimate tasks: 1: Bob

    section Afternoon
      Code review: 4: Alice, Bob
      Deploy: 5: Alice
```

## Gantt chart
```mermaid
gantt
    title Simple Gantt
    dateFormat  YYYY-MM-DD

    section Build
    Setup        :a1, 2026-01-01, 2d
    Implement    :a2, after a1, 5d
    Test         :a3, after a2, 3d
```

## Pie chart
```mermaid
pie
    title Pets adopted by volunteers
    "Dogs" : 386
    "Cats" : 85
    "Rats" : 15
```

## Quadrant chart
```mermaid
quadrantChart
    title Reach and engagement
    x-axis Low Reach --> High Reach
    y-axis Low Engagement --> High Engagement
    quadrant-1 High impact
    quadrant-2 Viral
    quadrant-3 Niche
    quadrant-4 Broad but shallow
    "Post A": [0.3, 0.6]
    "Post B": [0.8, 0.2]
```

## Timeline
```mermaid
timeline
    title History of Social Platforms
    2002 : LinkedIn
    2004 : Facebook
    2006 : Twitter
```

## Mindmap
```mermaid
mindmap
  root((mindmap))
    Origins
      Long history
    Research
      On effectiveness
    Tooling
      Mermaid
      Rust
```

## Git graph
```mermaid
gitGraph
    commit
    commit
    branch develop
    checkout develop
    commit
    checkout main
    merge develop
```

## Kanban board
```mermaid
kanban
Backlog
    Design system@{assigned: Alice, priority: High}
    API docs
Todo
    Auth flow@{priority: Very High}
    Dashboard@{assigned: Bob}
In Progress
    Login page@{assigned: Charlie, priority: Medium}
Done
    Setup CI@{assigned: Dave, priority: Low}
    Database schema
```

## Sankey diagram
```mermaid
sankey-beta
    A,B,10
    B,C,5
    B,D,5
```

## XY chart
```mermaid
xychart-beta
    title Demo
    x-axis 0 --> 10
    y-axis 0 --> 100
    line [5, 10, 20, 40]
```

## Requirement diagram
```mermaid
requirementDiagram
direction LR

requirement req1 {
    id: 1
    text: "The system shall do something"
    risk: high
    verifyMethod: test
}

element el1 {
    type: "Subsystem"
    docref: "DOC-1"
}

req1 - satisfies -> el1
```

## C4 context
```mermaid
C4Context
title System Context diagram for Internet Banking System
Person(customer, "Banking Customer", "A customer of the bank")
System(banking, "Internet Banking System", "Allows customers to view information about their bank accounts")
Rel(customer, banking, "Uses")
```

## Block diagram
```mermaid
block-beta
columns 2
A["Node A"]
B["Node B"]
C["Node C"]
D["Node D"]
```

## Packet diagram
```mermaid
packet-beta
0-3: "Header"
4-7: "Payload"
8: "CRC"
```

## Radar chart
```mermaid
radar-beta
axis A, B, C
curve Series1 { 1, 2, 3 }
```

## Info diagram
```mermaid
info
```

## Notes
- Verify that each code fence renders visually instead of as plain text.
- Check which diagram types fully render versus partially render or fall back.
- If useful, compare behavior between editor view, preview view, and reopen/resizing flows.
