I want a few improvements to this project:
  - I want this to be as easy as possible to install in any systems (mac, linux, windows). For Mac, I have the https://github.com/dbmrq/homebrew-tap repository,
  and you can use the gh cli to manage it.
  - I want the app to be able to run in a few modes: `paperboat "prompt"`, `paperboat path/to/plan/file`, plain `paperboat`. For plain `paperboat`, it should ask
  you for a prompt, and then proceed as usual. And this has to be handled properly with the backend choice (auggie/cursor).
  - The README should be extremely concise and it should highlight how to install and use the project, so users can try it as quickly as possible
  - The self-improvement agent should never change prompts
  - The "codecov" badge currently shows "unknown" in the readme
  - The readme should be updated with other basic info (while staying extremely concise), and anything that's outdated should be adjusted
  - We should thoroughly clean up the project and get it 100% ready to ship
  - Commits should be merged into fewer chunks with nice titles and descriptions (they're currently a mess). We don't need to put in much effort into this; it
  doesn't have to be super accurate. Just has to look good if anyone looks at the list of commits.