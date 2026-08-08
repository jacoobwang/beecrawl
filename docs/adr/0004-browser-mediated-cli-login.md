# Use Dashboard browser authorization for CLI login

`beecrawl login` will open the BeeCrawl Dashboard in the user's browser, let the user authenticate and authorize access, and then save the resulting BeeCrawl credential in a local CLI profile. The CLI will not collect Dashboard credentials directly; API-key and explicit endpoint flags remain available for automation and local development.
