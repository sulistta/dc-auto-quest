import React, { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
    Activity,
    AlertCircle,
    CheckCircle2,
    ChevronLeft,
    ChevronRight,
    Database,
    FileJson,
    Loader2,
    Play,
    Search,
    Square,
    Terminal
} from 'lucide-react'

import './app/global.css'

type GameExecutable = {
    arguments?: string
    is_launcher?: boolean
    name: string
    os: string
}

type GameListItem = {
    aliases?: string[]
    executables?: GameExecutable[]
    hook?: boolean
    id: string
    name: string
    overlay?: boolean
    themes?: string[]
}

type ProcessProfile = {
    aliases: string[]
    description: string
    executableName: string
    executablePath: string
    gameName: string
    hook: boolean
    id: string
    overlay: boolean
    processName: string
    themes: string[]
}

type SimulationStatus = {
    active: boolean
    pid: number | null
    target_name: string | null
    executable_path: string | null
}

const FALLBACK_PROFILES: ProcessProfile[] = [
    {
        aliases: [],
        description: 'Perfil local de fallback para validação da simulação.',
        executableName: 'processo_teste',
        executablePath: 'processo_teste',
        gameName: 'Processo de Teste',
        hook: false,
        id: 'fallback-processo-teste',
        overlay: false,
        processName: 'processo_teste',
        themes: ['Auditoria']
    }
]

const INACTIVE_STATUS: SimulationStatus = {
    active: false,
    pid: null,
    target_name: null,
    executable_path: null
}

const PROFILES_PER_PAGE = 24

function App() {
    const [profiles, setProfiles] =
        useState<ProcessProfile[]>(FALLBACK_PROFILES)
    const [selectedProfileId, setSelectedProfileId] = useState<string | null>(
        FALLBACK_PROFILES[0].id
    )
    const [selectedName, setSelectedName] = useState(
        FALLBACK_PROFILES[0].executablePath
    )
    const [status, setStatus] = useState<SimulationStatus>(INACTIVE_STATUS)
    const [isLoading, setIsLoading] = useState(false)
    const [message, setMessage] = useState(
        'Carregando lista detectavel do Discord...'
    )
    const [error, setError] = useState<string | null>(null)
    const [search, setSearch] = useState('')
    const [currentPage, setCurrentPage] = useState(1)
    const [dataSource, setDataSource] = useState('Discord detectable API')

    const selectedProfile = useMemo(
        () =>
            profiles.find((profile) => profile.id === selectedProfileId) ??
            null,
        [profiles, selectedProfileId]
    )

    const filteredProfiles = useMemo(() => {
        const query = search.trim().toLowerCase()

        if (!query) {
            return profiles
        }

        return profiles.filter((profile) =>
            [
                profile.gameName,
                profile.processName,
                profile.executableName,
                profile.executablePath,
                ...profile.aliases,
                ...profile.themes
            ]
                .join(' ')
                .toLowerCase()
                .includes(query)
        )
    }, [profiles, search])

    const totalPages = Math.max(
        1,
        Math.ceil(filteredProfiles.length / PROFILES_PER_PAGE)
    )
    const pageStart = (currentPage - 1) * PROFILES_PER_PAGE
    const pageEnd = Math.min(
        pageStart + PROFILES_PER_PAGE,
        filteredProfiles.length
    )
    const visibleProfiles = filteredProfiles.slice(pageStart, pageEnd)
    const validationMessage = validateTargetName(selectedName)
    const canStart =
        !isLoading &&
        !status.active &&
        selectedName.trim().length > 0 &&
        validationMessage === null

    useEffect(() => {
        let isMounted = true

        async function loadGameList() {
            try {
                const { games, source } = await loadDetectableGames()
                const nextProfiles = mapGameListToProfiles(games)

                if (nextProfiles.length === 0) {
                    throw new Error(
                        'lista detectável do Discord não contém executáveis válidos'
                    )
                }

                if (isMounted) {
                    setProfiles(nextProfiles)
                    setSelectedProfileId(nextProfiles[0].id)
                    setSelectedName(nextProfiles[0].executablePath)
                    setDataSource(source)
                    setMessage(
                        `${nextProfiles.length} executáveis carregados de ${source}.`
                    )
                }
            } catch (loadError) {
                if (isMounted) {
                    setError(formatError(loadError))
                    setMessage(
                        'Usando perfil de fallback porque a lista detectável do Discord não foi carregada.'
                    )
                }
            }
        }

        void loadGameList()

        return () => {
            isMounted = false
        }
    }, [])

    useEffect(() => {
        setCurrentPage(1)
    }, [search])

    useEffect(() => {
        setCurrentPage((page) => Math.min(page, totalPages))
    }, [totalPages])

    async function startSimulation() {
        setIsLoading(true)
        setError(null)

        try {
            const nextStatus = await invoke<SimulationStatus>(
                'start_simulation',
                {
                    gameTitle: selectedProfile?.gameName ?? selectedName,
                    targetName: selectedName
                }
            )
            setStatus(nextStatus)
            setMessage(
                `Simulação ativa para ${nextStatus.target_name ?? selectedName}.`
            )
        } catch (startError) {
            setError(formatError(startError))
        } finally {
            setIsLoading(false)
        }
    }

    async function stopSimulation() {
        setIsLoading(true)
        setError(null)

        try {
            const nextStatus = await invoke<SimulationStatus>('stop_simulation')
            setStatus(nextStatus)
            setMessage('Simulação encerrada e diretório temporário limpo.')
        } catch (stopError) {
            setError(formatError(stopError))
        } finally {
            setIsLoading(false)
        }
    }

    function selectProfile(profile: ProcessProfile) {
        setSelectedProfileId(profile.id)
        setSelectedName(profile.executablePath)
    }

    return (
        <main className="app-shell">
            <header className="app-titlebar">
                <div className="app-titleblock">
                    <span className="app-mark">DC</span>
                    <div>
                        <h1>DC Auto Quest</h1>
                        <p>
                            Simulador de processos baseado na lista detectável
                            do Discord
                        </p>
                    </div>
                </div>

                <div
                    className={`status-pill ${status.active ? 'is-active' : ''}`}
                >
                    {status.active ? (
                        <Activity aria-hidden="true" size={16} />
                    ) : (
                        <CheckCircle2 aria-hidden="true" size={16} />
                    )}
                    <span>{status.active ? 'Ativo' : 'Inativo'}</span>
                </div>
            </header>

            <section className="workspace-grid">
                <section
                    className="profiles-panel"
                    aria-labelledby="profiles-title"
                >
                    <div className="panel-header">
                        <div>
                            <h2 id="profiles-title">Perfis</h2>
                            <p>{profiles.length} perfis carregados</p>
                        </div>
                        <div className="count-strip">
                            {filteredProfiles.length > 0
                                ? `${pageStart + 1}-${pageEnd} de ${filteredProfiles.length}`
                                : '0 de 0'}
                        </div>
                    </div>

                    <label className="search-box" htmlFor="search">
                        <Search aria-hidden="true" size={17} />
                        <input
                            disabled={isLoading || status.active}
                            id="search"
                            onChange={(event) => setSearch(event.target.value)}
                            placeholder="Buscar por jogo, executavel ou tema"
                            value={search}
                        />
                    </label>

                    <div className="source-row">
                        <FileJson aria-hidden="true" size={16} />
                        <span>{dataSource}</span>
                    </div>

                    <div className="profile-list" role="list">
                        {visibleProfiles.length > 0 ? (
                            visibleProfiles.map((profile) => {
                                const isSelected =
                                    profile.id === selectedProfileId

                                return (
                                    <button
                                        aria-pressed={isSelected}
                                        className={`profile-row ${
                                            isSelected ? 'is-selected' : ''
                                        }`}
                                        disabled={isLoading || status.active}
                                        key={profile.id}
                                        onClick={() => selectProfile(profile)}
                                        type="button"
                                    >
                                        <span className="profile-main">
                                            <span className="profile-name">
                                                {profile.gameName}
                                            </span>
                                            <span className="profile-process">
                                                {profile.processName}
                                            </span>
                                        </span>
                                        <span className="profile-tags">
                                            {profile.themes
                                                .slice(0, 3)
                                                .map((theme) => (
                                                    <span
                                                        key={`${profile.id}-${theme}`}
                                                    >
                                                        {theme}
                                                    </span>
                                                ))}
                                        </span>
                                    </button>
                                )
                            })
                        ) : (
                            <div className="empty-state">
                                Nenhum perfil encontrado para a busca atual.
                            </div>
                        )}
                    </div>
                    <nav
                        className="pagination-bar"
                        aria-label="Paginacao de perfis"
                    >
                        <button
                            aria-label="Pagina anterior"
                            disabled={currentPage === 1}
                            onClick={() =>
                                setCurrentPage((page) => Math.max(1, page - 1))
                            }
                            type="button"
                        >
                            <ChevronLeft aria-hidden="true" size={16} />
                            <span>Anterior</span>
                        </button>
                        <span>
                            Pagina {currentPage} de {totalPages}
                        </span>
                        <button
                            aria-label="Proxima pagina"
                            disabled={currentPage === totalPages}
                            onClick={() =>
                                setCurrentPage((page) =>
                                    Math.min(totalPages, page + 1)
                                )
                            }
                            type="button"
                        >
                            <span>Proxima</span>
                            <ChevronRight aria-hidden="true" size={16} />
                        </button>
                    </nav>
                </section>

                <aside
                    className="control-panel"
                    aria-labelledby="control-title"
                >
                    <div className="panel-header">
                        <div>
                            <h2 id="control-title">Simulacao</h2>
                            <p>Uma simulacao ativa por vez</p>
                        </div>
                        <Terminal aria-hidden="true" size={20} />
                    </div>

                    <div className="detail-block">
                        <InfoRow
                            label="Jogo"
                            value={selectedProfile?.gameName ?? 'Customizado'}
                        />
                        <InfoRow
                            label="Executavel original"
                            value={selectedProfile?.executableName ?? '-'}
                            mono
                        />
                        <InfoRow
                            label="Nome simulado"
                            value={status.target_name ?? (selectedName || '-')}
                            mono
                        />
                        <InfoRow
                            label="PID"
                            value={status.pid?.toString() ?? '-'}
                            mono
                        />
                        <InfoRow
                            label="Binario temporario"
                            value={status.executable_path ?? '-'}
                            mono
                        />
                    </div>

                    <div className="field-group">
                        <label htmlFor="target-name">
                            Caminho do executavel a simular
                        </label>
                        <input
                            disabled={isLoading || status.active}
                            id="target-name"
                            onChange={(event) => {
                                setSelectedProfileId(null)
                                setSelectedName(event.target.value)
                            }}
                            placeholder="game.exe ou bin/game.exe"
                            spellCheck={false}
                            value={selectedName}
                        />
                        <p
                            className={
                                validationMessage ? 'field-error' : 'field-help'
                            }
                        >
                            {validationMessage ??
                                'Use caminho relativo com letras, numeros, ponto, hifen, underscore ou barra.'}
                        </p>
                    </div>

                    <div className="actions-row">
                        <button
                            className="primary-action"
                            disabled={!canStart}
                            onClick={startSimulation}
                            type="button"
                        >
                            {isLoading && !status.active ? (
                                <Loader2
                                    aria-hidden="true"
                                    className="spin"
                                    size={17}
                                />
                            ) : (
                                <Play aria-hidden="true" size={17} />
                            )}
                            <span>
                                {isLoading && !status.active
                                    ? 'Iniciando'
                                    : 'Iniciar'}
                            </span>
                        </button>
                        <button
                            className="danger-action"
                            disabled={isLoading || !status.active}
                            onClick={stopSimulation}
                            type="button"
                        >
                            {isLoading && status.active ? (
                                <Loader2
                                    aria-hidden="true"
                                    className="spin"
                                    size={17}
                                />
                            ) : (
                                <Square aria-hidden="true" size={15} />
                            )}
                            <span>
                                {isLoading && status.active
                                    ? 'Parando'
                                    : 'Parar'}
                            </span>
                        </button>
                    </div>

                    <div className="message-block">
                        <Database aria-hidden="true" size={17} />
                        <span>{message}</span>
                    </div>

                    {error ? (
                        <div className="error-block">
                            <AlertCircle aria-hidden="true" size={17} />
                            <span>{error}</span>
                        </div>
                    ) : null}
                </aside>
            </section>
        </main>
    )
}

async function loadDetectableGames() {
    const payload = await invoke<string>('fetch_detectable_games')

    return {
        games: JSON.parse(payload) as GameListItem[],
        source: 'Discord detectable API'
    }
}

function mapGameListToProfiles(games: GameListItem[]) {
    return games.flatMap((game, index) => {
        const executables = pickExecutables(game.executables)

        if (executables.length === 0) {
            return []
        }

        const themes = game.themes ?? []
        const aliases = game.aliases ?? []

        return executables.map((executable, executableIndex) => {
            const executablePath = toSafeExecutablePath(
                executable.name,
                game.id
            )
            const processName =
                executablePath.split('/').pop() ?? executablePath

            return {
                aliases,
                description: buildDescription(game, executable, themes),
                executableName: executable.name,
                executablePath,
                gameName: game.name,
                hook: Boolean(game.hook),
                id: `${game.id}-${index}-${executableIndex}`,
                overlay: Boolean(game.overlay),
                processName,
                themes
            }
        })
    })
}

function pickExecutables(executables: GameExecutable[] | undefined) {
    if (!executables || executables.length === 0) {
        return []
    }

    const windowsExecutables = executables.filter(
        (executable) => executable.os === 'win32' && !executable.is_launcher
    )

    if (windowsExecutables.length > 0) {
        return dedupeExecutables(windowsExecutables)
    }

    return dedupeExecutables(
        executables.filter((executable) => !executable.is_launcher)
    )
}

function dedupeExecutables(executables: GameExecutable[]) {
    const seen = new Set<string>()

    return executables.filter((executable) => {
        const key = executable.name.toLowerCase()

        if (seen.has(key)) {
            return false
        }

        seen.add(key)
        return true
    })
}

function buildDescription(
    game: GameListItem,
    executable: GameExecutable,
    themes: string[]
) {
    const flags = [game.hook ? 'hook' : null, game.overlay ? 'overlay' : null]
        .filter(Boolean)
        .join(' / ')
    const themeText =
        themes.length > 0
            ? themes.slice(0, 4).join(', ')
            : 'sem temas cadastrados'
    const flagText = flags ? ` Recursos: ${flags}.` : ''

    return `Executável base: ${executable.name}. Temas: ${themeText}.${flagText}`
}

function toSafeExecutablePath(rawName: string, gameId: string) {
    const normalizedParts = rawName
        .replace(/^>+/, '')
        .replace(/\\/g, '/')
        .split('/')
        .map((part) => sanitizePathPart(part))
        .filter(Boolean)

    const normalized = normalizedParts.join('/').slice(0, 180)

    return normalized || `game_${gameId}`.slice(0, 80)
}

function sanitizePathPart(rawPart: string) {
    const asciiOnly = Array.from(rawPart.trim().normalize('NFKD'))
        .filter((char) => char.charCodeAt(0) <= 127)
        .join('')
    const normalized = asciiOnly
        .replace(/[^a-zA-Z0-9._-]+/g, '_')
        .replace(/_+/g, '_')
        .replace(/^[_.-]+|[_.-]+$/g, '')
        .slice(0, 80)

    return normalized
}

function validateTargetName(targetName: string) {
    const trimmed = targetName.trim()

    if (!trimmed) {
        return 'Informe um nome de processo.'
    }

    if (trimmed.length > 180) {
        return 'Use no maximo 180 caracteres.'
    }

    if (trimmed.startsWith('/') || trimmed.startsWith('\\')) {
        return 'Use um caminho relativo.'
    }

    const parts = trimmed.replace(/\\/g, '/').split('/')

    if (
        parts.some(
            (part) =>
                part.length === 0 ||
                part === '.' ||
                part === '..' ||
                part.endsWith('.')
        )
    ) {
        return 'Use apenas partes de caminho validas.'
    }

    if (parts.some((part) => !/^[a-zA-Z0-9._-]+$/.test(part))) {
        return 'Use apenas letras, numeros, ponto, hifen, underscore ou barra.'
    }

    if (parts.some(isWindowsReservedName)) {
        return 'Uma parte do caminho e reservada pelo Windows.'
    }

    return null
}

function isWindowsReservedName(name: string) {
    const stem = name.split('.')[0].toUpperCase()

    return [
        'CON',
        'PRN',
        'AUX',
        'NUL',
        'COM1',
        'COM2',
        'COM3',
        'COM4',
        'COM5',
        'COM6',
        'COM7',
        'COM8',
        'COM9',
        'LPT1',
        'LPT2',
        'LPT3',
        'LPT4',
        'LPT5',
        'LPT6',
        'LPT7',
        'LPT8',
        'LPT9'
    ].includes(stem)
}

function InfoRow({
    label,
    mono = false,
    value
}: {
    label: string
    mono?: boolean
    value: React.ReactNode
}) {
    return (
        <div className="info-row">
            <span>{label}</span>
            <strong className={mono ? 'is-mono' : undefined}>{value}</strong>
        </div>
    )
}

function formatError(error: unknown) {
    return error instanceof Error ? error.message : String(error)
}

export default App
