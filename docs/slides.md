# Slides

Five slide decks, one for each audience. Each one is a web page that you can
present in a browser, and a PDF in 16:9. [All the decks on one page](slides/index.html).

| Deck | For | Web | PDF |
| --- | --- | --- | --- |
| An overview | technical people who do not know chaps | [open](slides/overview.html) | [PDF](slides/overview.pdf) |
| The chap CLI, without Python | people who use `uvx --from chap-core chap` | [open](slides/chap-cli-users.html) | [PDF](slides/chap-cli-users.pdf) |
| Chap behind the Modeling App | people who use DHIS2 and the Modeling App | [open](slides/modeling-app-users.html) | [PDF](slides/modeling-app-users.pdf) |
| A workshop | students, in a hands-on session of about one hour | [open](slides/workshop.html) | [PDF](slides/workshop.pdf) |
| Developing a model | people who write forecasting models | [open](slides/model-developers.html) | [PDF](slides/model-developers.pdf) |

In the web page, the arrow keys move between the slides, and `f` shows the
deck on the full screen.

The decks are Markdown files in `slides/decks/`, with one theme in
`slides/theme/chaps.css`. To change one, edit its file and run `make slides`.
See [Development](./development.md#slides).
