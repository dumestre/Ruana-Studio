# Canal de builds de teste

O Ruana Hub consulta o manifesto público em:

`https://builds.daime.space/ruana/stable/channel.json`

Os arquivos serão armazenados em um bucket Cloudflare R2. Para os links usados
pelo Hub funcionarem, o bucket precisa estar público e o domínio
`builds.daime.space` precisa estar conectado a ele na Cloudflare. Não coloque
credenciais ou chaves da Cloudflare no Hub ou nos arquivos publicados.

Compile o Ruana Studio como de costume. O script usa `target/release/`; para
usar outra pasta, informe `-BuildFolder`. Na raiz do repositório, execute:

```powershell
.\ruana-hub\scripts\package-test-build.ps1 -Version "0.2.0-beta.1"
```

O script não compila nem envia a build. Ele gera dois arquivos em
`target/ruana-test-channel/<versão>/`:

- `channel.json`: versão, link público do ZIP e SHA-256;
- `RuanaStudio-<versão>-windows-x64.zip`: executável e assets de cursor.

Envie os dois arquivos para o caminho `ruana/stable/` no bucket R2. O endereço
do ZIP no manifesto será
`https://builds.daime.space/ruana/stable/<nome-do-zip>`. O Hub valida o domínio
HTTPS, o caminho, o formato da versão e o SHA-256 antes de extrair o pacote.
A atualização substitui a instalação anterior somente depois de validar e
preparar a nova build.

O manifesto e o ZIP precisam estar acessíveis por HTTPS sem login para que os
testadores baixem pelo Hub. Se o domínio `daime.space` ainda não estiver
configurado na Cloudflare, configure-o antes de publicar builds; não altere os
nameservers sem preservar os registros DNS atuais do site.
