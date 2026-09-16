# Post para o Reddit: Slot2Sync

> **Rascunho para publicar depois.** O texto descreve um estado que ainda não é o atual. Conferir
> antes de publicar:
>
> 1. **Android.** O texto cita a versão Android, que ainda não está implementada nem publicada em
>    release (o job `android` do `release.yml` está com `if: false` até os secrets do keystore
>    serem configurados).
> 2. **Validação com o Google.** O texto não menciona o aviso de "app não verificado" porque o
>    domínio próprio e a validação junto ao Google estão previstos para antes da publicação. Se
>    isso não estiver concluído, o aviso vermelho aparece no login e precisa voltar ao texto.
>
> Sugestão de subs: r/emulation, r/EmuDev, r/opensource, r/software (ajustar o título/corpo
> conforme as regras de auto-promoção de cada um; alguns exigem prefixo "[Self-Promo]" ou só
> permitem em dias/threads específicos).

## Título

Slot2Sync: app open source que sincroniza saves de emulador via provedor de nuvem (como Google
Drive) ou pasta local

## Corpo

Slot2Sync é um app para desktop (Windows/Linux) e Android que sincroniza saves e savestates
de emuladores. Hoje funciona com Google Drive ou com uma pasta local/de rede, sem depender de
nuvem. A arquitetura é agnóstica de provedor, então Dropbox e OneDrive estão previstos.

O problema que ele resolve: o save fica na pasta de um aparelho só. Ao trocar de PC, formatar ou
continuar o jogo no celular, o progresso não acompanha, e a alternativa é copiar as pastas na
mão.

O escopo é só save de emulador. A prioridade do projeto é não perder progresso do usuário.

**Feito para quem não é técnico:**  
O objetivo do projeto é que alguém sem familiaridade com tecnologia consiga usar. Não tem
terminal, não tem arquivo de configuração pra editar na mão, não tem servidor ou container pra
subir e não tem nada pra deixar rodando em outra máquina. A instalação é o instalador normal da
sua plataforma, e a configuração é escolher o provedor e confirmar o emulador que o app detectou.
A interface está em inglês e português.

**Como funciona:**

- Conecta o provedor que preferir (Google Drive ou Local, que estão disponíveis no momento) e aponta a pasta do emulador (se o emulador estiver instalado na pasta padrão, ele já é reconhecido automaticamente e te mostra a lista de todos os emuladores que você tiver instalado, bastando selecionar o que deseja).
- Depois disso, ele sincroniza sozinho quando você inicia e encerra o emulador. No desktop fica na bandeja do sistema.
- O mesmo save vai do PC pro celular e do celular pro PC, é só usar a mesma conta nos dois.
- Sem servidor pra manter, sem precisar de dois dispositivos ligados ao mesmo tempo, sem terminal.
- Uma vez que o arquivo sobe, ele nunca é apagado no provedor, só atualizado. Perdeu o PC, formatou, o HD morreu, a cópia continua lá.
- Se duas máquinas mexeram no mesmo save, você decide qual manter.
- Funciona offline, sincroniza quando tiver internet.

**Como funciona o backup:**
Antes de qualquer sync sobrescrever um arquivo local, a versão atual é arquivada num histórico
local, com carimbo de data/hora, separado por emulador e categoria (saves e savestates).
Dá pra listar esse histórico e restaurar uma versão anterior. A quantidade de versões mantidas e
o tempo de retenção são configuráveis. O app não implementa nenhuma
operação de exclusão. Não existe caminho de código que apague um arquivo lá, dá pra conferir no
repositório.

**Acesso e privacidade:**
No seu Google Drive o app só enxerga os arquivos que ele mesmo criou. O resto da sua conta fica invisível
pra ele, não dá pra listar nem abrir nada que o Slot2Sync não tenha colocado lá. Nos outros
provedores funciona igual, cada um com uma pasta separada só do app.

Seus saves vão direto do seu dispositivo para a sua conta. Não tem servidor no meio, não tem
telemetria e não tem conta pra criar do meu lado. A única parte que passa por fora é o login, que
usa um proxy no Cloudflare pra que a conexão com a sua conta seja possível e segura. Ele
participa só do login, nunca dos seus arquivos.

Deixei isso resumido de propósito. Quem quiser entender a parte técnica a fundo, a documentação
explica a arquitetura, as decisões de projeto e como o login funciona por dentro.

**Emuladores suportados hoje:** PPSSPP e PCSX2. Adicionar um novo emulador é criar um perfil
novo, sem mexer no resto do app, e é uma área boa pra quem quiser contribuir.

**Limites atuais:**

- O login e a renovação de acesso ao Google passam pelo proxy no Cloudflare, que hoje roda no plano gratuito, com teto de 100 mil requisições por dia. O gasto por pessoa é baixo, mas o teto existe.
- O modo de pasta local não depende de nada disso. Sem login, sem proxy, sem serviço meu no caminho.
- Projeto 1.0, mantido por uma pessoa só. Vai ter bug, e a velocidade de resposta é a minha disponibilidade.

**Disclaimer:**
O Slot2Sync não tem a intenção de substituir o Syncthing nem outras ferramentas de versionamento
de arquivos, que são bem mais maduras. Em vários problemas de lógica eu me baseei no próprio
Syncthing pra resolver. A proposta é ser uma opção mais rápida e simples de aplicar, sem servidor,
sem depender de dois dispositivos ligados ao mesmo tempo e sem ter que seguir tutorial longo. Pra
quem só queria conectar os saves e não montar uma malha de infraestrutura de versionamento, é um
caminho mais acessível.

**Sobre o uso de IA:**
Sou desenvolvedor, hoje focado em DevOps, e usei IA de forma assistida na construção do projeto.
Prefiro deixar isso claro de saída. A IA entrou como ferramenta, não como autor: arquitetura,
decisões técnicas e revisão são minhas, e a responsabilidade pelo que está no repositório também.

O projeto também é estudo. Estou me profissionalizando em desenvolvimento com IA agêntica: a empresa onde trabalho se tornou a primeira parceira da Anthropic no Brasil, e foi por aí que veio o acesso a uma certificação oficial deles na área. Usei o MVP do Slot2Sync como projeto prático dessa certificação. Ou seja, o cuidado com o uso da ferramenta é parte do que eu estava treinando, não um detalhe.

Por exemplo, o núcleo de sincronização não conhece emulador nem provedor de armazenamento, os dois entram por trait, então dá pra adicionar qualquer um dos dois sem tocar no resto. O backend tem mais de 400 testes, que rodam no CI junto com clippy e fmt. Se você achar alguma decisão mal resolvida, abra uma issue que eu explico o motivo ou corrijo.

Licença GPL-3.0, código no GitHub. Ainda em versão pré-1.0, então feedback, bugs e sugestões são bem-vindos.

- GitHub: https://github.com/jIDvDIj/slot2sync
- Como instalar e usar: https://jidvdij.github.io/slot2sync-site/docs/
- Documentação técnica (arquitetura e decisões de projeto): https://jidvdij.github.io/slot2sync-site/docs/dev/
