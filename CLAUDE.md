@AGENTS.md

Never compile, build or lint on this machine (`cargo`, `tauri`, `tsc`, `vite build`, bundlers, `npm run build`, `npm run verify`,
`npm install` of a git dependency that runs `prepare`): GitHub Actions does it. Only an explicit request of the owner, in that
request, to build or test locally overrides this. See "Nothing is compiled on this machine" in AGENTS.md.

Ogni app della suite ha l'unico aspetto di Sigrid: solo componenti di Sigrid, mai un componente, una classe o un foglio
di stile propri; ciò che manca si aggiunge a Sigrid. Vedi «Un solo aspetto per tutta la suite» in AGENTS.md.
