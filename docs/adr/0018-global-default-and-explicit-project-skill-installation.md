# Make global Skill installation the default

`beecrawl init --all` will install the Skill into detected users' global Agent skill directories. Project-local installation will require `--scope project`, keeping the common setup available across repositories while making repository mutations explicit for teams that want a checked-in Skill.
