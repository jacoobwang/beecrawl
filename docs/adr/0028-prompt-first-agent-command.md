# Make the Agent command prompt-first

`beecrawl agent` will take a required prompt as its primary input, accept repeated `--url` source hints and a `--max-credits` budget, and support prompt files or stdin for long instructions. The command remains blocking by default and shares the explicit Job lifecycle subcommands used by other long-running operations.
