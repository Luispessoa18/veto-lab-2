# AVAL Console

Painel do operador do Aval: mostra o Aval julgando as transações dos agentes em tempo real e transforma os vereditos em monitoramento.

## Telas

| Tela | O que mostra |
|---|---|
| **Live gate** (inicial) | Agentes enviam transações; cada uma passa pelas seis etapas do Aval (interceptar, simular, ler efeitos, origem dos endereços, checagens, decisão) e recebe ALLOW / FLAG / DENY. Cada veredito é uma chamada real ao motor; as etapas são reexibidas em câmera lenta e o tempo real do motor aparece no carimbo. |
| **Overview** | Totais da sessão e as intervenções do gate (negados e retidos). |
| **Lab traffic** | Requisições julgadas pela API deste repositório (`src/unified_api.py`, porta 8070), lidas do log de auditoria (`GET /admin/requests`) a cada 2 s: decisão, camada que parou, caminho pelas camadas (blacklist → Prompt Guard → IA → simulação) e latência. Clique numa linha para ver o motivo de cada camada. Se o benchmark Solana já rodou (`09_SIMULAR_ATAQUES_SOLANA`), o resumo de `GET /admin/report` aparece no topo. |
| **Issues** | Vereditos repetidos agrupados por regra + agente (como o Sentry agrupa erros): contagem, tendência, primeira/última vez, dono, resolver/ignorar. Um issue resolvido que volta a acontecer aparece como "came back". |
| **Actions** | Log de todos os vereditos com visão detalhada por período (15 min, 1 h, sessão): volume, latência do motor, mix de decisões, regras e agentes; arraste sobre um gráfico para filtrar o log por janela de tempo. Cada ação abre o trace completo, com a **linha do tempo** do que aconteceu (o que o agente leu → propôs → simulação → origem → checagens → onde foi parado). |
| **Monitoring, Review, Scenario Lab, Agents, Policy, Manifests, Lists, Deploy, System** | Gráficos, fila de revisão, laboratório de cenários e configuração. |

## Rodar

O Live gate, Issues e Actions usam o **servidor de demonstração do Veto** (porta 5173), que não faz parte deste repositório. **Lab traffic** e **System** funcionam só com este repositório: a API (`./iniciar_tudo.sh` ou `00_INICIAR_TUDO.bat`, que também sobem o console) e o aval-svm.

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
