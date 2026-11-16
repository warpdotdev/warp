let jackLastNameArgument = {
    name: "last-name",
    description: "Last name of the jack",
    values: [{ value: "Nichols" }, { value: "Nicholson" }],
};
let generatedArgument = {
    name: "generated-name",
    description: "Last name of the jack",
    values: [
        {
            generateSuggestionsFn: { script: "echo foobar" }
        }
    ],
};
export function activate(warp) {
    warp.completions.registerCommandSignature({
        command: {
            name: "jack",
            alias: "j",
            description: "Jack's special command",
            subcommands: [
                {
                    name: "create",
                    description: "Create a jack :D.",
                    arguments: jackLastNameArgument,
                },
                {
                    name: "delete",
                    description: "Delete a jack :(.",
                    arguments: jackLastNameArgument,
                },
                {
                    name: "update",
                    description: "Update a jack",
                    arguments: [
                        jackLastNameArgument,
                    ]
                },
                {
                    name: "generate",
                    description: "Generate a jack",
                    arguments: [
                        generatedArgument
                    ]
                },
            ],
            options: [
                {
                    name: ["-s", "--special"],
                    description: "Only operate on special jacks",
                    arguments: {
                        name: "level",
                        description: "Level of specialness",
                        values: [{ value: "1" }, { value: "2" }, { value: "3" }],
                    }
                },
                {
                    name: ["-c", "--cracker"],
                    description: "Only operate on cracker jacks",
                },
                {
                    name: "--eat",
                    description: "Eat the jacks",
                    requiredOptions: ["-c"],
                },
            ],
        },
    });
    console.log("Successfully registered command signatures!");
}
