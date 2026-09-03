1. Arquitetura Definid

Esqueleto da UI (Slint UI): Responsável por toda a interface ao redor da área de edição. Isso inclui os controles de transporte (Play, Stop, Record), faders de volume na mesa de som, menus de contexto, splash screens, atalhos de teclado e navegador de arquivos/plugins.

Motor da Timeline (wgpu): O centro da tela será um canvas renderizado nativamente na GPU via wgpu, embutido dentro da janela do Slint. A timeline não usará componentes de UI padrão, mas sim técnicas de engine de jogos 2D.

1.

 Especificações da Timeline (wgpu)
A timeline deve ser tratada como um ambiente gráfico de alta performance com os seguintes pilares:

Câmera Ortográfica 2D: Implementar uma matriz matemática de projeção para lidar com Zoom in/out e Pan (rolagem) de forma independente do tamanho e quantidade dos clipes, garantindo fluidez.

Instancing Render: Desenhar milhares de clipes de áudio e as linhas de grade de tempo usando renderização instanciada (enviando buffers de posição/tamanho/cor para um único retângulo base).

Waveforms via Shaders: Desenhar as formas de onda diretamente no Fragment Shader, lendo os dados de picos de áudio de uma Textura 1D ou Storage Buffer, permitindo interpolação instantânea com o zoom.

Interatividade (Hit-Testing em CPU): Manter uma estrutura de dados matemática (ex: R-Tree) para calcular colisões (AABB) de cliques do mouse na timeline. Isso controlará estados de seleção, arrasto de clipes (drag-and-drop com snap magnético no grid) e redimensionamento das bordas (trimming).

1. Sincronização e Fluxo de Dados
Comunicação lock-free entre a thread de DSP (áudio) e o backend de UI/Render.

Sincronização em tempo real da posição do cursor de reprodução (playhead) e atualização dos medidores de volume (VU meters) enviados para o Slint.

Passagem correta dos buffers de dados visuais do áudio para o pipeline do wgpu.

1. Sua Tarefa Agora
Com base nessa arquitetura, gere o código de fundação (scaffolding) do projeto. Preciso de:

A estrutura básica do arquivo .slint contendo o layout principal (transporte, área do mixer) e a declaração de um elemento customizado que receberá a textura/superfície do wgpu.

O código Rust principal (ex: main.rs) configurando a janela do Slint.

A estrutura inicial do módulo wgpu (setup do device, queue e criação do pipeline de renderização) preparado para ser injetado no Slint.

Gere o código modular, bem documentado e preparado para escalar. Foque na integração técnica entre o Slint e a superfície wgpu como primeiro passo.
