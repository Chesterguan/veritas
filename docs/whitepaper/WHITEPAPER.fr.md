# VERITAS — Environnement d'Exécution d'Agents de Confiance

**Livre blanc v0.2**

*Déterministe · Auditable · Vérifiable*

> Dernière mise à jour : 2026-02-17

---

## Table des matières

1. [Vision](#1-vision)
2. [Motivation](#2-motivation)
3. [Philosophie de conception](#3-design-philosophy)
4. [Filiation de conception ZeroClaw](#4-zeroclaw-design-lineage)
5. [Modèle système](#5-system-model)
6. [Modèle de confiance](#6-trust-model)
7. [Modèle de capacités](#7-capability-model)
8. [Politique et gouvernance](#8-policy-and-governance)
9. [Audit et traçabilité](#9-audit-and-traceability)
10. [Modèle de vérification](#10-verification-model)
11. [Modèle de sécurité](#11-security-model)
12. [Indépendance du modèle de données](#12-data-model-independence)
13. [Extensibilité](#13-extensibility)

---

## 1. Vision

VERITAS est un environnement d'exécution déterministe, contraint par des politiques, auditable et vérifiable pour les agents IA opérant dans des environnements réglementés. Le système privilégie la confiance, le contrôle et la preuve sur l'autonomie et l'intelligence opaque.

VERITAS est conçu comme une couche d'exécution fondamentale plutôt que comme un produit applicatif ou d'automatisation.

## 2. Motivation

Les agents IA modernes sont puissants mais fondamentalement peu fiables dans les environnements réglementés. Leur comportement est souvent non déterministe, non auditable et difficile à vérifier.

VERITAS répond à ces limitations en fournissant un environnement d'exécution contrôlé où chaque décision, action et sortie est traçable et contrainte par des politiques.

## 3. Philosophie de conception

Les dix principes qui gouvernent toutes les décisions de conception de VERITAS :

1. **Contrôle sur l'autonomie**
2. **Preuve sur l'intelligence**
3. **Déterminisme sur l'émergence**
4. **Refus par défaut**
5. **Sécurité fondée sur les capacités**
6. **Base de calcul de confiance minimale**
7. **Auditabilité par conception**
8. **Exécution vérifiable**
9. **Possibilité de substitution humaine en tout temps**
10. **Indépendance du modèle de données**

## 4. Filiation de conception ZeroClaw

VERITAS s'appuie sur la philosophie légère et déterministe de ZeroClaw. ZeroClaw met l'accent sur un noyau d'agent minimal, un flux d'exécution explicite, la composabilité et une base de calcul de confiance réduite.

VERITAS étend cette philosophie en introduisant l'application des politiques, l'auditabilité, la vérification et des frontières d'exécution sécurisées, tout en préservant la conception légère et modulaire.

## 5. Modèle système

L'exécution d'agents dans VERITAS est modélisée comme une machine à états déterministe opérant sur des capacités contrôlées.

### Boucle d'exécution

```
State → Policy → Capability → Audit → Verify → Next State
```

Chaque transition est explicite, vérifiée par politique, auditée et validée avant que l'agent ne passe à l'état suivant.

## 6. Modèle de confiance

La confiance dans VERITAS est dérivée de l'exécution déterministe, des pistes d'audit immuables, des décisions de politique explicites et des sorties vérifiables.

Le système ne fait pas confiance intrinsèquement au raisonnement des LLM, aux outils externes, aux données d'entrée, ni aux environnements d'exécution.

### Frontière de confiance

| Fiable | Non fiable |
|---------|-----------|
| Noyau du runtime | LLM |
| Moteur de politique | Outils |
| Moteur d'audit | Données d'entrée |
| Vérificateur | Environnement externe |

## 7. Modèle de capacités

Les capacités représentent des outils contraints avec des schémas, des permissions et des déclarations d'effets secondaires explicites.

Toutes les interactions avec le monde extérieur doivent passer par des capacités sous contrôle de politique.

## 8. Politique et gouvernance

VERITAS applique une exécution avec refus par défaut. Les décisions de politique évaluent le sujet, l'action, la ressource et le contexte pour déterminer l'un des trois résultats suivants :

- **Autoriser**
- **Refuser**
- **Nécessite une approbation**

La politique est déterministe, explicable et auditable.

## 9. Audit et traçabilité

Tous les événements d'exécution sont enregistrés dans un flux d'événements en ajout seul formant un graphe d'exécution vérifiable. Chaque événement contient :

- Les transitions d'état
- Les appels de capacités
- Les décisions de politique
- Les résultats de vérification

Le système prend en charge des traces d'exécution rejouables et inviolables.

## 10. Modèle de vérification

Toutes les sorties doivent passer des contrôles de validation incluant :

- La validation de schéma
- La validation de règles
- L'évaluation des risques

Une vérification secondaire optionnelle et une révision humaine peuvent être requises pour les opérations sensibles.

## 11. Modèle de sécurité

VERITAS applique le moindre privilège, l'exécution isolée des capacités et un contrôle strict des frontières.

Le runtime ne permet pas l'accès direct au système et considère que tous les composants externes sont non fiables.

## 12. Indépendance du modèle de données

Le runtime principal de VERITAS est indépendant des modèles de données de santé ou d'entreprise spécifiques tels que FHIR, OMOP ou les schémas propriétaires.

Les adaptateurs spécifiques au domaine sont implémentés en externe via des capacités.

## 13. Extensibilité

VERITAS fournit des interfaces standardisées pour :

- Les capacités
- Les moteurs de politique
- Le stockage d'audit
- Les modules de vérification

Les contributeurs externes peuvent étendre le système sans modifier le noyau de confiance.

---

*Fin du livre blanc VERITAS v0.2*
