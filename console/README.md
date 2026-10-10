# AVAL Console

Painel do operador do Aval: mostra o Aval julgando as transações dos agentes em tempo real e transforma os vereditos em monitoramento.

## Telas

| Tela | O que mostra |
|---|---|
| **Live gate** (inicial) | Agentes enviam transações; cada uma passa pelas seis etapas do Aval (interceptar, simular, ler efeitos, origem dos endereços, checagens, decisão) e recebe ALLOW / FLAG / DENY. Cada veredito é uma chamada real ao motor; as etapas são reexibidas em câmera lenta e o tempo real do motor aparece no carimbo. |
| **Overview** | Totais da sessão e as intervenções do gate (negados e retidos). |
| **Issues** | Vereditos repetidos agrupados por regra + agente (como o Sentry agrupa erros): contagem, tendência, primeira/última vez, dono, resolver/ignorar. Um issue resolvido que volta a acontecer aparece como "came back". |
| **Actions** | Log de todos os vereditos, com gráfico no topo: arraste sobre o gráfico para filtrar o log por janela de tempo. Cada ação abre o trace completo, com a **linha do tempo** do que aconteceu (o que o agente leu → propôs → simulação → origem → checagens → onde foi parado). |
| **Monitoring, Review, Scenario Lab, Agents, Policy, Manifests, Lists, Deploy, System** | Gráficos, fila de revisão, laboratório de cenários e configuração. |

## Rodar

Precisa do **servidor de demonstração do Veto** rodando (porta 5173). Ele não faz parte deste repositório.

```bash
cd console
npm install
npm run dev        # http://127.0.0.1:5190
```

Os back ends são acessados por proxy (`vite.config.ts`), sem CORS:

| Caminho | Destino | Variável |
|---|---|---|
| `/veto` | servidor de demonstração do Veto (`127.0.0.1:5173`) | `VETO_URL` |
| `/lab` | API deste repositório (`127.0.0.1:8070`) | `LAB_URL` |
| `/svm` | aval-svm (`127.0.0.1:8899`) | `SVM_URL` |

Sem o motor, o console abre normalmente e diz **engine offline**; nada é inventado.

## O que não está aqui

- **Saídas gravadas do motor**: esta cópia não traz snapshots; tudo vem do motor ao vivo.
- **Manifestos de ferramentas**: vêm do motor. A tela Manifests explica isso quando a lista está vazia.
- Configuração (políticas, listas, agentes) e ações nos issues ficam no navegador (`localStorage`); só a allowlist e a rede são enviadas ao motor a cada avaliação. O resto fica marcado como *staged* até o motor ter uma API de configuração.

## Licenças de terceiros

Gráficos portados do **Monocharts** (MIT), licença em `src/components/ui/mono/LICENSE-monocharts.txt`.
