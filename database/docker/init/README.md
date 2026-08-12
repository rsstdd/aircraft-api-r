# PostgreSQL container initialization

Files in this directory run only when the Docker PostgreSQL volume is empty.

Do not place normal schema migrations here.

Use this directory only for one-time bootstrap logic that must exist before migrations,
such as optional local roles or database-level setup.