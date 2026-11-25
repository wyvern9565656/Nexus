# NEXUS

NEXUS é um gerenciador de arquivos para Linux com ferramentas para pesquisar arquivos e entender o uso do armazenamento. Ele combina um explorador gráfico com um índice local SQLite e comandos de terminal.

## O que ele faz

- Navega por pastas em visualização de grade ou lista, com busca na pasta, ordenação, favoritos e prévia de arquivos.
- Copia, recorta e cola arquivos e pastas, incluindo operações por atalho e pelo menu de contexto.
- Compacta itens em ZIP ou tar.gz e extrai `.zip`, `.tar.gz`, `.tgz` e `.tar`, sempre preservando os originais.
- Executa arquivos executáveis pelo menu de contexto, incluindo scripts `.sh`/`.bash` e `.AppImage`.
- Identifica arquivos por tipo e por linguagem, com reconhecimento de nomes especiais como `Cargo.toml`, `Dockerfile`, `package.json`, `CMakeLists.txt`, `Makefile`, `README` e `LICENSE`.
- Pesquisa no índice por nome, caminho, extensão e tamanho; é possível limitar a pesquisa à pasta atual.
- Varre pastas e calcula tamanhos lógicos e ocupação em disco.
- Mostra o uso do espaço em uma árvore visual, lista arquivos grandes e encontra arquivos duplicados.
- Envia arquivos para a lixeira do usuário. A ação sobre duplicatas só é habilitada para grupos verificados.

## Requisitos

- Linux e Rust com Cargo.
- `xdg-open` para abrir arquivos e pastas no aplicativo padrão.
- `xdg-user-dir` para localizar as pastas pessoais (Documentos, Downloads, Imagens etc.).
- `ffmpeg` para prévia e miniaturas de vídeo.
- `xclip` é usado como alternativa para acesso ao clipboard em sessões X11; em Wayland, o aplicativo também usa a integração nativa disponível.

## Compilar e executar

Na pasta do projeto:

```bash
cargo build --release
./target/release/nexus
```

Sem subcomando, o Nexus abre a interface gráfica. Também é possível iniciá-la explicitamente:

```bash
./target/release/nexus gui
```

Para uma compilação de desenvolvimento, use `cargo build` e execute `./target/debug/nexus`.

## Interface gráfica

O explorador abre na pasta pessoal. Use a barra de navegação para trocar de pasta, alternar entre grade e lista, pesquisar dentro da pasta e abrir a prévia. O menu do botão direito em um item oferece ações para aquele arquivo ou pasta; no espaço vazio da pasta, ele oferece ações para o destino atual, incluindo colar.

Ao clicar com o botão direito em uma pasta, **Colar em «pasta»** cola o conteúdo dentro dela. O mesmo menu traz **Compactar em ZIP**, **Compactar em tar.gz**, **Extrair aqui** (para arquivos compatíveis) e **Executar** (para arquivos executáveis). Compactar e extrair rodam em segundo plano e nunca alteram nem removem os itens originais.

Os atalhos principais são:

| Atalho | Ação |
| --- | --- |
| `Ctrl+C` | Copiar seleção |
| `Ctrl+X` | Recortar seleção |
| `Ctrl+V` | Colar na pasta atual |
| `Ctrl+A` | Selecionar todos os itens |
| `Ctrl+K` ou `Ctrl+P` | Abrir a paleta de comandos |
| `Backspace`, `Left` ou `H` | Subir para a pasta pai |
| `Enter` | Abrir o item selecionado |
| `Space` | Abrir a prévia rápida |
| `/` | Focar a busca da pasta |

As telas de pesquisa e armazenamento usam o índice do Nexus. Na tela de armazenamento, selecione uma pasta e escolha **Escanear** para criar ou atualizar seus dados. As abas **Espaço**, **Duplicatas** e **Maiores** apresentam análises diferentes desse índice.

## Ícones

Os ícones seguem uma cadeia de prioridade: primeiro nomes especiais (`Cargo.toml`, `Dockerfile`, `package.json`, `docker-compose.yml`, `CMakeLists.txt`, `Makefile`, `README*`, `LICENSE*`, `.git`, entre outros), depois a extensão e, por fim, um ícone genérico com a extensão. Linguagens comuns têm ícone próprio: Python, C, C++, Rust, JavaScript, TypeScript, JVM (Java/Kotlin/Scala), Go, Shell, Web (HTML/CSS), PHP, Ruby, Swift, Dart, R, Haskell, Elixir e Lua.

Imagens suportadas usam a própria imagem como miniatura. Sobre qualquer ícone pode aparecer um emblema independente: corrente para link simbólico, cadeado para arquivo sem permissão de leitura, play para executável e um véu suave para arquivos ocultos.

## Comandos de terminal

Todos os comandos aceitam `--db <caminho>` para escolher outro arquivo de índice. Por padrão, o banco fica em `~/.local/share/nexus/index.db` (ou em `$XDG_DATA_HOME/nexus/index.db`, se definido).

```bash
# Abrir a interface
nexus

# Abrir como seletor de arquivos (imprime o(s) caminho(s) escolhido(s))
nexus pick [pasta]
nexus pick ~/Downloads --multiple

# Indexar ou atualizar uma pasta
nexus scan ~/Downloads

# Pesquisar no índice por nome e extensão
nexus search "relatorio" --ext pdf

# Mostrar os maiores arquivos do índice
nexus top --n 30

# Encontrar duplicatas, ignorando arquivos menores que 1 KiB
nexus dupes --min-size 1024 --limit 20
```

Para ver todas as opções de um comando, use `nexus --help` ou `nexus <comando> --help`.

## Dados e privacidade

<<<<<<< HEAD
O índice é um banco SQLite local. A pesquisa e as análises descritas neste README usam esse índice; execute uma varredura para atualizar as informações depois de mudanças nos arquivos. O Nexus não precisa de uma conta ou serviço remoto para essas funções.⏎                
=======
O índice é um banco SQLite local. A pesquisa e as análises descritas neste README usam esse índice; execute uma varredura para atualizar as informações depois de mudanças nos arquivos. O Nexus não precisa de uma conta ou serviço remoto para essas funções.
>>>>>>> 73b81ad (Initial commit: Nexus app)
